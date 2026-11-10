//! Read-only Paper eligibility over independently reopened source, Raw, Daily,
//! and SQL0066 evidence. This module exposes no Paper activation or write path.

#[cfg(target_os = "linux")]
pub mod checked_admission;

#[cfg(all(test, target_os = "linux"))]
mod integration_tests;

/// Data-only financial input capture over the same checker custody path.
pub mod financial;

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Utc};
use domain::{ContentHash, TradingDate, UtcTimestamp};
use market_data::{
    ResponseKind, SourceFileReference,
    daily_price_only::{
        DailyActionChainV1Wire, DailyCalendarBindingV1Wire, DailyPricePinsV1Wire,
        DailySessionEvidenceV1Wire, DailySessionTypeV1,
        calendar::{VerifiedCalendarProjection, eligible_calendar_date},
        source::ScanLimitValues,
        verify::{RawEvidenceSnapshots, resolve_original},
    },
};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    contract::{
        ELIGIBILITY_TTL_SECONDS, PaperEligibilityDecision, PaperEligibilityReasonCode as Reason,
        PaperEligibilityReport, UntrustedPaperContractV1, denied,
    },
    input::UntrustedPaperSessionReadbackV1,
    source_evidence::{self as wire, ProtectedSourceChain, UntrustedCurrentEnvelope},
};
use crate::owner_beta::runner::daily_ingest::{
    DAILY_ADMITTER_UID, DAILY_READER_GROUP, DailyIngestConfig,
};

const SOURCE_EVENTS_ROOT: &str = "/data/owner-beta-paper-evidence/events";
const RAW_MANIFEST_BYTES: usize = 16_384;
const MAX_CALENDAR_DAYS: u64 = 369;
// Mirrors calendar.rs MAX_ROW_CLAIMS, which is private to that module.
const MAX_VERIFIED_CALENDAR_CLAIMS: usize = 262_144;
const MAX_SESSIONS: usize = 3;
const CLOCK_DIVERGENCE_TOLERANCE: Duration = Duration::from_secs(1);
const CHECKER_TRANSACTION_SETUP: &str = include_str!("read_transaction.sql");
const DAILY_CONFIG_ALIAS: &str = "/run/owner-beta-daily/config.json";
const DAILY_POLICY_ALIAS: &str = "/run/owner-beta-daily/policy.json";
const DAILY_RAW_ALIAS: &str = "/data/raw";
const DAILY_ADMITTED_ALIAS: &str = "/data/owner-beta-daily/admitted";
const DAILY_HISTORICAL_ALIAS: &str = "/data/owner-beta-artifacts";

struct EvaluationClock {
    started_at: DateTime<Utc>,
    previous_wall: DateTime<Utc>,
    started_monotonic: Instant,
    eligible_date: TradingDate,
}

impl EvaluationClock {
    fn start() -> Result<Self, Reason> {
        let started_at = Utc::now();
        #[cfg(all(test, target_os = "linux"))]
        let started_at = integration_tests::start_override().unwrap_or(started_at);
        let eligible_date = eligible_calendar_date(UtcTimestamp::from_datetime(started_at))
            .map_err(|_| Reason::CalendarUnproven)?;
        Ok(Self {
            started_at,
            previous_wall: started_at,
            started_monotonic: Instant::now(),
            eligible_date,
        })
    }

    #[cfg(test)]
    fn synthetic(started_at: DateTime<Utc>) -> Result<Self, Reason> {
        let eligible_date = eligible_calendar_date(UtcTimestamp::from_datetime(started_at))
            .map_err(|_| Reason::CalendarUnproven)?;
        Ok(Self {
            started_at,
            previous_wall: started_at,
            started_monotonic: Instant::now(),
            eligible_date,
        })
    }

    fn checkpoint(&mut self) -> Result<DateTime<Utc>, Reason> {
        let wall_now = Utc::now();
        let monotonic_elapsed = self.started_monotonic.elapsed();
        #[cfg(all(test, target_os = "linux"))]
        let wall_now = integration_tests::sample_override(self.started_at, monotonic_elapsed)
            .unwrap_or(wall_now);
        self.check_sample(wall_now, monotonic_elapsed)
    }

    fn check_sample(
        &mut self,
        wall_now: DateTime<Utc>,
        monotonic_elapsed: Duration,
    ) -> Result<DateTime<Utc>, Reason> {
        let ttl = Duration::from_secs(
            u64::try_from(ELIGIBILITY_TTL_SECONDS).map_err(|_| Reason::EligibilityExpired)?,
        );
        if wall_now < self.previous_wall {
            return Err(Reason::EligibilityExpired);
        }
        let wall_elapsed = wall_now
            .signed_duration_since(self.started_at)
            .to_std()
            .map_err(|_| Reason::EligibilityExpired)?;
        if wall_elapsed >= ttl || monotonic_elapsed >= ttl {
            return Err(Reason::EligibilityExpired);
        }
        let divergence = wall_elapsed.abs_diff(monotonic_elapsed);
        if divergence > CLOCK_DIVERGENCE_TOLERANCE {
            return Err(Reason::EligibilityExpired);
        }
        let eligible_date = eligible_calendar_date(UtcTimestamp::from_datetime(wall_now))
            .map_err(|_| Reason::CalendarUnproven)?;
        if eligible_date != self.eligible_date {
            return Err(Reason::EligibilityExpired);
        }
        self.previous_wall = wall_now;
        Ok(wall_now)
    }
}

/// Evaluate the latest three verified XKRX trading sessions. The binding is an
/// untrusted selector: success requires protected source chains, current
/// worker-only SQL0066 readbacks, retained Raw originals, and sealed Daily
/// admissions to agree. The returned report is descriptive and cannot activate
/// Paper; a consumer must rerun this check immediately before any future use.
pub async fn check_paper_eligibility(
    pool: &PgPool,
    binding: &UntrustedPaperContractV1,
) -> PaperEligibilityReport {
    let mut clock = match EvaluationClock::start() {
        Ok(clock) => clock,
        Err(reason) => return denied(reason, Utc::now()),
    };
    match check_paper_eligibility_at(pool, binding, &mut clock).await {
        Ok(report) => report,
        Err(reason) => denied(reason, Utc::now()),
    }
}

// Bound holders are awaited locally; this library future does not promise Send.
async fn check_paper_eligibility_at(
    pool: &PgPool,
    binding: &UntrustedPaperContractV1,
    clock: &mut EvaluationClock,
) -> Result<PaperEligibilityReport, Reason> {
    #[cfg(target_os = "linux")]
    {
        checker_custody::run(pool, binding, clock, None, None, None, None)
            .await
            .publish()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pool, binding, clock);
        Err(Reason::ReaderIncompatible)
    }
}

async fn read_paper_calculation_inputs_at(
    pool: &PgPool,
    binding: &UntrustedPaperContractV1,
    request: &financial::PaperCalculationInputRequest,
    clock: &mut EvaluationClock,
) -> Result<financial::ObservedPaperCalculationInputs, financial::PaperCalculationInputError> {
    #[cfg(target_os = "linux")]
    {
        checker_custody::run(pool, binding, clock, Some(request), None, None, None)
            .await
            .publish_capture(request)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pool, binding, request, clock);
        Err(financial::PaperCalculationInputError::Eligibility(
            Reason::ReaderIncompatible,
        ))
    }
}

async fn read_checked_paper_action_inputs_at(
    pool: &PgPool,
    binding: &UntrustedPaperContractV1,
    request: &financial::PaperCalculationInputRequest,
    action_request: &financial::PaperActionInputRequest,
    outer_revalidate: &mut dyn financial::ActionContextGuard,
    consumer: &mut dyn financial::LocalActionConsumer,
) -> ActionCapturePublication {
    #[cfg(target_os = "linux")]
    {
        let mut clock = match EvaluationClock::start() {
            Ok(clock) => clock,
            Err(reason) => return ActionCapturePublication::refused(reason),
        };
        checker_custody::run(
            pool,
            binding,
            &mut clock,
            Some(request),
            Some(action_request),
            Some(outer_revalidate),
            Some(consumer),
        )
        .await
        .publish_action()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            pool,
            binding,
            request,
            action_request,
            outer_revalidate,
            consumer,
        );
        ActionCapturePublication::refused(Reason::ReaderIncompatible)
    }
}

/// Native ENABLE can consume the same retained checker traversal as ordinary
/// actions without loading the serving helper's runtime identity. The guard is
/// supplied by the root-only caller and remains live through SQL commit and
/// checker cleanup.
#[cfg(target_os = "linux")]
pub(crate) trait NativePaperActionGuard {
    fn revalidate(&mut self) -> Result<(), financial::PaperInputActionError>;
    fn checkpoint(&mut self) -> Result<(), financial::PaperInputActionError>;
}

#[cfg(target_os = "linux")]
pub(crate) trait NativePaperActionConsumer {
    fn consume<'a, 'hold>(
        &'a mut self,
        action: &'a mut financial::CheckedPaperAction<'hold>,
    ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>>
    where
        'hold: 'a;
}

#[cfg(target_os = "linux")]
pub(crate) struct NativePaperActionCompletion {
    pub(crate) first: Option<financial::PaperInputActionError>,
    pub(crate) secondary: Vec<financial::PaperInputActionError>,
    pub(crate) committed: bool,
    pub(crate) commit_started: bool,
}

#[cfg(target_os = "linux")]
struct NativeGuardBridge<'a>(&'a mut dyn NativePaperActionGuard);

#[cfg(target_os = "linux")]
impl financial::ActionContextGuard for NativeGuardBridge<'_> {
    fn revalidate(&mut self) -> Result<(), financial::PaperInputActionError> {
        self.0.revalidate()
    }

    fn checkpoint(&mut self) -> Result<(), financial::PaperInputActionError> {
        self.0.checkpoint()
    }
}

#[cfg(target_os = "linux")]
struct NativeConsumerBridge<'a>(&'a mut dyn NativePaperActionConsumer);

#[cfg(target_os = "linux")]
impl financial::LocalActionConsumer for NativeConsumerBridge<'_> {
    fn invoke<'a>(
        &'a mut self,
        action: &'a mut financial::CheckedPaperAction<'_>,
    ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
        self.0.consume(action)
    }
}

/// Run ENABLE inside the genuine source/current/rights checker and expose only
/// a borrowed checked action to the consuming callback. The candidate witness
/// is derived from the protected Current snapshot inside checker custody.
#[cfg(target_os = "linux")]
pub(crate) async fn with_native_checked_paper_action(
    pool: &PgPool,
    guard: &mut dyn NativePaperActionGuard,
    consumer: &mut dyn NativePaperActionConsumer,
) -> NativePaperActionCompletion {
    let mut clock = match EvaluationClock::start() {
        Ok(clock) => clock,
        Err(reason) => {
            return NativePaperActionCompletion {
                first: Some(financial::PaperInputActionError::Input(reason)),
                secondary: Vec::new(),
                committed: false,
                commit_started: false,
            };
        }
    };
    let selector = financial::PaperActionInputRequest::LatestSignal;
    let capture = selector.calculation_request();
    let mut guard = NativeGuardBridge(guard);
    let mut consumer = NativeConsumerBridge(consumer);
    let outcome = checker_custody::run_selected(
        pool,
        None,
        None,
        &mut clock,
        Some(&capture),
        Some(&selector),
        Some(&mut guard),
        Some(&mut consumer),
    )
    .await
    .publish_action();
    NativePaperActionCompletion {
        first: outcome.first,
        secondary: outcome.secondary,
        committed: outcome.committed,
        commit_started: outcome.commit_started,
    }
}

/// A borrowed native traversal retains this actual SQL transaction, not a
/// caller token or row list. Its only query rejoins the protected decision to
/// immutable originals and current authority. Root/Current/Source remain
/// independently owned by the surrounding native/checker scopes.
#[cfg(target_os = "linux")]
struct NativeRecoveryReadContext {
    transaction: tokio::sync::Mutex<sqlx::Transaction<'static, sqlx::Postgres>>,
    decision: Value,
}

#[cfg(target_os = "linux")]
impl NativeRecoveryReadContext {
    async fn read(&self, date: NaiveDate, cohort: &Value) -> Result<Value, sqlx::Error> {
        let mut transaction = self.transaction.lock().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            sqlx::query_scalar("SELECT public.paper_0068_read_recovery_session($1,$2,$3)")
                .bind(&self.decision)
                .bind(date)
                .bind(cohort)
                .fetch_one(&mut **transaction),
        )
        .await
        .map_err(|_| sqlx::Error::Protocol("recovery reader deadline".into()))?
    }

    async fn close(self) -> bool {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.transaction.into_inner().rollback(),
        )
        .await
        .is_ok_and(|result| result.is_ok())
    }
}

/// RECOVER selects immutable T/run and exact E from the protected SQL original.
/// Selectors confer no grant: the callback must compare borrowed checked facts
/// and consume current authority in its retained SQL transaction.
#[cfg(target_os = "linux")]
#[allow(
    clippy::too_many_arguments,
    reason = "Separate bounded privileged reader and worker custody remain explicit."
)]
pub(crate) async fn with_native_checked_recovery_action(
    pool: &PgPool,
    recovery_pool: &PgPool,
    decision: &Value,
    selector: &financial::PaperActionInputRequest,
    signal_cohort: Value,
    execution_cohort: Value,
    guard: &mut dyn NativePaperActionGuard,
    consumer: &mut dyn NativePaperActionConsumer,
) -> NativePaperActionCompletion {
    let rejected = |first| NativePaperActionCompletion {
        first: Some(first),
        secondary: Vec::new(),
        committed: false,
        commit_started: false,
    };
    if !matches!(
        selector,
        financial::PaperActionInputRequest::Settle { .. }
            | financial::PaperActionInputRequest::Value { .. }
    ) {
        return rejected(financial::PaperInputActionError::TransactionState);
    }
    let mut capture = selector.calculation_request();
    if let Err(error) = capture.select_recovery_cohorts(signal_cohort, execution_cohort) {
        return rejected(error);
    }
    let mut clock = match EvaluationClock::start() {
        Ok(clock) => clock,
        Err(reason) => return rejected(financial::PaperInputActionError::Input(reason)),
    };
    let mut transaction =
        match tokio::time::timeout(std::time::Duration::from_secs(10), recovery_pool.begin()).await
        {
            Ok(Ok(transaction)) => transaction,
            _ => return rejected(financial::PaperInputActionError::Sql),
        };
    // READ COMMITTED gives each repeated statement a fresh snapshot while
    // retaining current rights/config/source locks through grant COMMIT/readback.
    // This transaction only reads; FOR SHARE requires a read-write transaction.
    let setup = sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await;
    if setup.is_err() {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(5), transaction.rollback()).await;
        return rejected(financial::PaperInputActionError::Sql);
    }
    let reader = NativeRecoveryReadContext {
        transaction: tokio::sync::Mutex::new(transaction),
        decision: decision.clone(),
    };
    let mut guard = NativeGuardBridge(guard);
    let mut consumer = NativeConsumerBridge(consumer);
    let mut outcome = checker_custody::run_selected(
        pool,
        Some(&reader),
        None,
        &mut clock,
        Some(&capture),
        Some(selector),
        Some(&mut guard),
        Some(&mut consumer),
    )
    .await
    .publish_action();
    if !reader.close().await {
        if outcome.first.is_none() {
            outcome.first = Some(financial::PaperInputActionError::Sql);
        } else {
            outcome
                .secondary
                .push(financial::PaperInputActionError::Sql);
        }
    }
    if let Err(reason) = clock.checkpoint() {
        if outcome.first.is_none() {
            outcome.first = Some(financial::PaperInputActionError::Input(reason));
        } else {
            outcome
                .secondary
                .push(financial::PaperInputActionError::Input(reason));
        }
    }
    NativePaperActionCompletion {
        first: outcome.first,
        secondary: outcome.secondary,
        committed: outcome.committed,
        commit_started: outcome.commit_started,
    }
}

struct ActionCapturePublication {
    first: Option<financial::PaperInputActionError>,
    secondary: Vec<financial::PaperInputActionError>,
    committed: bool,
    commit_started: bool,
    retained: Vec<financial::RetainedActionValue>,
}

impl ActionCapturePublication {
    fn refused(reason: Reason) -> Self {
        Self {
            first: Some(financial::PaperInputActionError::Input(reason)),
            secondary: Vec::new(),
            committed: false,
            commit_started: false,
            retained: Vec::new(),
        }
    }
}

fn authoritative_binding_from_snapshot(
    snapshot: &UntrustedCurrentEnvelope,
    candidate: &UntrustedPaperContractV1,
) -> Result<Value, Reason> {
    if snapshot.status().map_err(|_| Reason::CohortChanged)? != "CURRENT" {
        return Err(Reason::CohortChanged);
    }
    let authoritative = snapshot
        .value()
        .get("binding")
        .filter(|value| !value.is_null())
        .ok_or(Reason::CohortChanged)?;
    if authoritative != candidate.as_untrusted_value() {
        return Err(Reason::CohortChanged);
    }
    Ok(authoritative.clone())
}

struct DailyContext {
    config: DailyIngestConfig,
    data_root: PathBuf,
    raw_owner_uid: u32,
    scan_limits: ScanLimitValues,
    entitlement_reference: String,
    policy_hash: ContentHash,
    owner: Uuid,
}

impl DailyContext {
    fn from_selection_with(
        session: &mut ConfigSession<'_, '_>,
        partial: &mut ContextPartial,
        binding: &Value,
        selection: ConfigSelection,
    ) -> Result<Self, ContextConfigFailure> {
        session.preflight().map_err(ContextConfigFailure::Read)?;
        let (config_path, pin_text, environment_revision) = match selection {
            ConfigSelection::ServingEnvironment => (
                std::env::var_os("PRICE_BETA_DAILY_CONFIG_FILE")
                    .map(PathBuf::from)
                    .ok_or(Reason::ReaderIncompatible)?,
                std::env::var("PRICE_BETA_DAILY_CONFIG_SHA256")
                    .map_err(|_| Reason::ReaderIncompatible)?,
                std::env::var("LAGRANGE_CODE_COMMIT").ok(),
            ),
            // Selected only by the private native checker entry point AFTER
            // reopening genuine Current. No caller flag or ambient path/pin.
            ConfigSelection::NativeCurrent => (
                PathBuf::from(DAILY_CONFIG_ALIAS),
                string(binding, "daily_config_sha256")?.to_owned(),
                option_env!("LAGRANGE_CODE_COMMIT").map(str::to_owned),
            ),
        };
        let config_path_text = config_path.to_str().ok_or(Reason::ReaderIncompatible)?;
        if config_path_text != DAILY_CONFIG_ALIAS {
            return Err(ContextConfigFailure::Reason(Reason::ReaderIncompatible));
        }
        let config_pin = ContentHash::parse(&pin_text).map_err(|_| Reason::ReaderIncompatible)?;
        if pin_text != binding["daily_config_sha256"] {
            return Err(ContextConfigFailure::Reason(Reason::CohortChanged));
        }
        // Validate the fixed policy and data aliases before the established
        // typed reader follows settings.policy_path to open its pinned policy.
        let index = context_config_bytes(session, &config_path, &config_pin)?;
        partial.settings = Some(
            serde_json::from_slice(session.bytes(index)).map_err(|_| Reason::EvidenceTampered)?,
        );
        let settings = partial.settings.as_ref().expect("real first settings");
        partial.config = Some(after_config_alias(
            || {
                validate_daily_alias_mapping(
                    binding,
                    settings,
                    config_path_text,
                    &pin_text,
                    environment_revision.as_deref(),
                )
                .map_err(ContextConfigFailure::from)
            },
            || {
                DailyIngestConfig::from_pinned_file_with(
                    session,
                    &mut partial.typed,
                    &config_path,
                    &config_pin,
                )
                .map_err(ContextConfigFailure::Ingest)
            },
        )?);

        let owner =
            Uuid::parse_str(string(settings, "owner")?).map_err(|_| Reason::OwnerScopeMismatch)?;
        let policy_hash = ContentHash::parse(string(settings, "policy_sha256")?)
            .map_err(|_| Reason::CohortChanged)?;
        let data_root = PathBuf::from("/data");
        let raw_owner_uid = u32::try_from(
            settings["raw_owner_uid"]
                .as_u64()
                .ok_or(Reason::ReaderIncompatible)?,
        )
        .map_err(|_| Reason::ReaderIncompatible)?;
        let scan_limits = parse_scan_limits(&settings["scan"])?;
        let entitlement_reference = string(settings, "entitlement_reference")?.to_owned();

        Ok(Self {
            config: partial.config.take().expect("real loaded config"),
            data_root,
            raw_owner_uid,
            scan_limits,
            entitlement_reference,
            policy_hash,
            owner,
        })
    }
}

fn validate_daily_alias_mapping(
    binding: &Value,
    settings: &Value,
    config_path: &str,
    config_pin: &str,
    environment_revision: Option<&str>,
) -> Result<(), Reason> {
    if config_path != DAILY_CONFIG_ALIAS {
        return Err(Reason::ReaderIncompatible);
    }
    if binding["daily_config_sha256"] != config_pin
        || settings["policy_path"] != DAILY_POLICY_ALIAS
        || settings["raw_root"] != DAILY_RAW_ALIAS
        || settings["admitted_root"] != DAILY_ADMITTED_ALIAS
        || settings["historical_root"] != DAILY_HISTORICAL_ALIAS
    {
        return Err(Reason::CohortChanged);
    }

    let raw_owner_uid = settings["raw_owner_uid"]
        .as_u64()
        .ok_or(Reason::ReaderIncompatible)?;
    if binding["raw_owner_uid"].as_u64() != Some(raw_owner_uid) {
        return Err(Reason::CohortChanged);
    }
    let cohort = &binding["cohort"];
    if cohort["owner_user_id"] != settings["owner"] {
        return Err(Reason::OwnerScopeMismatch);
    }
    if cohort["policy_sha256"] != settings["policy_sha256"]
        || cohort["entitlement_reference"] != settings["entitlement_reference"]
        || cohort["entitlement_sha256"] != settings["entitlement_sha256"]
    {
        return Err(Reason::RightsUnavailable);
    }

    let revision = string(cohort, "verifier_revision")?;
    let release_root = string(binding, "release_root")?;
    if settings["code_commit"] != revision
        || environment_revision != Some(revision)
        || !release_root.ends_with(&format!("/{revision}"))
        || binding["daily_config_file"] != format!("{release_root}/.owner-beta-daily/config.json")
        || binding["release_manifest_file"] != format!("{release_root}/.lagrange-release-manifest")
    {
        return Err(Reason::CohortChanged);
    }
    Ok(())
}

fn read_pinned_daily_config(path: &Path, expected: &ContentHash) -> Result<Vec<u8>, Reason> {
    let mut session = ConfigSession::legacy();
    let index = read_pinned_daily_config_with(&mut session, path, expected)
        .map_err(ConfigFailure::legacy_source)?;
    Ok(session.take_legacy_bytes(index))
}
fn read_pinned_daily_config_with(
    session: &mut ConfigSession<'_, '_>,
    path: &Path,
    expected: &ContentHash,
) -> Result<usize, ConfigFailure<Reason>> {
    session.preflight()?;
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        session.with_file(
            ConfigReadStage::AliasConfig,
            path,
            |_| Reason::ReaderIncompatible,
            |file, bytes, check| {
                check()?;
                let before = file.metadata().map_err(|_| Reason::ReaderIncompatible)?;
                if !path.is_absolute()
                    || path == Path::new("/")
                    || alias_config_len_invalid(before.len())
                    || !before.is_file()
                    || before.nlink() != 1
                    || before.mode() & 0o022 != 0
                {
                    return Err(Reason::ReaderIncompatible.into());
                }
                check()?;
                Read::by_ref(file)
                    .take(RAW_MANIFEST_BYTES as u64 + 1)
                    .read_to_end(bytes)
                    .map_err(|_| Reason::ReaderIncompatible)?;
                check()?;
                let after = file.metadata().map_err(|_| Reason::ReaderIncompatible)?;
                if bytes.len() as u64 != before.len()
                    || before.len() != after.len()
                    || before.mode() != after.mode()
                    || before.uid() != after.uid()
                    || before.gid() != after.gid()
                    || before.nlink() != after.nlink()
                    || before.ctime() != after.ctime()
                    || before.ctime_nsec() != after.ctime_nsec()
                    || before.mtime() != after.mtime()
                    || before.mtime_nsec() != after.mtime_nsec()
                    || ContentHash::from_bytes(bytes) != *expected
                {
                    return Err(Reason::CohortChanged.into());
                }
                Ok(())
            },
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, expected);
        Err(session.non_linux_error(Reason::ReaderIncompatible))
    }
}

#[cfg(any(target_os = "linux", test))]
fn alias_config_len_invalid(len: u64) -> bool {
    len == 0 || len > RAW_MANIFEST_BYTES as u64
}
fn after_config_alias<T, E>(
    validate: impl FnOnce() -> Result<(), E>,
    load: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    validate()?;
    load()
}

// BEGIN C9 contextual config retention
use crate::owner_beta::runner::daily_ingest::{
    ConfigCompletion, ConfigFailure, ConfigFirstKind, ConfigReadStage, ConfigSession, IngestError,
    TypedConfigRetention,
};
enum ContextConfigFailure {
    Reason(Reason),
    Read(ConfigFailure<Reason>),
    Ingest(ConfigFailure<IngestError>),
}
impl From<Reason> for ContextConfigFailure {
    fn from(source: Reason) -> Self {
        Self::Reason(source)
    }
}
impl ContextConfigFailure {
    fn kind(&self) -> ConfigFirstKind {
        match self {
            Self::Reason(_) => ConfigFirstKind::Policy,
            Self::Read(source) => source.kind(),
            Self::Ingest(source) => source.kind(),
        }
    }
    fn reason(&self) -> Option<&Reason> {
        match self {
            Self::Reason(source) => Some(source),
            Self::Read(source) => source.policy(),
            Self::Ingest(_) => None,
        }
    }
    fn ingest(&self) -> Option<&IngestError> {
        match self {
            Self::Ingest(source) => source.policy(),
            _ => None,
        }
    }
    fn report<'a>(
        &self,
        completion: &'a ConfigCompletion,
    ) -> Option<&'a market_data::checked_close::Report> {
        match self {
            Self::Reason(_) => None,
            Self::Read(source) => source.report(completion),
            Self::Ingest(source) => source.report(completion),
        }
    }
}
impl std::fmt::Debug for ContextConfigFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextConfigFailure")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}
#[derive(Default)]
struct ContextPartial {
    settings: Option<Value>,
    config: Option<DailyIngestConfig>,
    typed: TypedConfigRetention,
}
#[derive(Default)]
struct ContextLoadRetention {
    partials: Vec<ContextPartial>,
    renewed: Vec<DailyContext>,
    selection: ConfigSelection,
}

#[derive(Clone, Copy, Default)]
enum ConfigSelection {
    #[default]
    ServingEnvironment,
    NativeCurrent,
}
impl ContextLoadRetention {
    fn partial_count(&self) -> usize {
        self.partials
            .iter()
            .map(|p| {
                usize::from(p.settings.is_some())
                    + usize::from(p.config.is_some())
                    + p.typed.partial_count()
            })
            .sum()
    }
}
fn retain_context_revalidation<T, E>(
    current: T,
    compare: impl FnOnce(&T) -> Result<(), E>,
    retained: &mut Vec<T>,
) -> Result<(), E> {
    let result = compare(&current);
    retained.push(current);
    result
}
impl DailyContext {
    fn load_in(
        session: &mut ConfigSession<'_, '_>,
        retained: &mut ContextLoadRetention,
        binding: &Value,
    ) -> Result<Self, ContextConfigFailure> {
        let mut partial = ContextPartial::default();
        let result = Self::from_selection_with(session, &mut partial, binding, retained.selection);
        retained.partials.push(partial);
        result
    }
    fn revalidate_in(
        &self,
        session: &mut ConfigSession<'_, '_>,
        retained: &mut ContextLoadRetention,
        binding: &Value,
    ) -> Result<(), ContextConfigFailure> {
        let current = Self::load_in(session, retained, binding)?;
        retain_context_revalidation(
            current,
            |current| compare_daily_context(self, current).map_err(ContextConfigFailure::from),
            &mut retained.renewed,
        )
    }
}
fn context_config_bytes(
    session: &mut ConfigSession<'_, '_>,
    path: &Path,
    expected: &ContentHash,
) -> Result<usize, ContextConfigFailure> {
    if session.is_legacy() {
        let bytes = read_pinned_daily_config(path, expected)?;
        Ok(session.legacy_bytes(ConfigReadStage::AliasConfig, bytes))
    } else {
        read_pinned_daily_config_with(session, path, expected).map_err(ContextConfigFailure::Read)
    }
}
// END C9 contextual config retention

fn compare_daily_context(previous: &DailyContext, current: &DailyContext) -> Result<(), Reason> {
    if current.data_root != previous.data_root
        || current.raw_owner_uid != previous.raw_owner_uid
        || current.scan_limits != previous.scan_limits
        || current.entitlement_reference != previous.entitlement_reference
        || current.policy_hash != previous.policy_hash
        || current.owner != previous.owner
        || current.config.policy().sha256() != previous.config.policy().sha256()
        || current.config.admission_path() != previous.config.admission_path()
        || current.config.historical_path() != previous.config.historical_path()
        || current.config.replay_limits() != previous.config.replay_limits()
    {
        return Err(Reason::CohortChanged);
    }
    Ok(())
}

fn selected_entitlement_batches<T>(
    batches: impl IntoIterator<Item = (bool, T)>,
) -> impl Iterator<Item = T> {
    batches
        .into_iter()
        .filter_map(|(matches, batch)| matches.then_some(batch))
}

fn latest_trading_candidates(
    verified_days: impl IntoIterator<Item = (NaiveDate, DailySessionTypeV1)>,
    eligible_date: NaiveDate,
) -> Result<[NaiveDate; MAX_SESSIONS], Reason> {
    let floor = eligible_date
        .checked_sub_days(chrono::Days::new(MAX_CALENDAR_DAYS))
        .ok_or(Reason::CalendarUnproven)?;
    let candidates = verified_days
        .into_iter()
        .filter_map(|(date, kind)| {
            (date >= floor && date <= eligible_date && kind == DailySessionTypeV1::Trading)
                .then_some(date)
        })
        .collect::<BTreeSet<_>>();
    if candidates.len() < MAX_SESSIONS {
        return Err(Reason::CalendarUnproven);
    }
    let mut selected = candidates
        .into_iter()
        .rev()
        .take(MAX_SESSIONS)
        .collect::<Vec<_>>();
    selected.reverse();
    selected.try_into().map_err(|_| Reason::CalendarUnproven)
}

fn prove_verified_calendar_suffix<P: CalendarClaims>(
    projections: &[&P],
    start: TradingDate,
    end: TradingDate,
) -> Result<Vec<(NaiveDate, DailySessionTypeV1)>, Reason> {
    // prove_calendar_range merges conflicts from every date in each projection,
    // including dates outside the required suffix. These are already verified
    // public projection facts; scope only their merge here and keep its row cap.
    if projections.is_empty() || projections.len() > MAX_VERIFIED_CALENDAR_CLAIMS {
        return Err(Reason::CalendarUnproven);
    }
    let first = projections[0];
    let span = end
        .as_naive_date()
        .signed_duration_since(start.as_naive_date())
        .num_days();
    if !(0..i64::try_from(MAX_CALENDAR_DAYS + 1).map_err(|_| Reason::CalendarUnproven)?)
        .contains(&span)
    {
        return Err(Reason::CalendarUnproven);
    }

    let mut total_claims = 0usize;
    let mut facts = std::collections::BTreeMap::new();
    for projection in projections {
        if projection.owner() != first.owner()
            || projection.policy_sha256() != first.policy_sha256()
        {
            return Err(Reason::CalendarUnproven);
        }
        total_claims = total_claims
            .checked_add(projection.days().len())
            .ok_or(Reason::CalendarUnproven)?;
        if total_claims > MAX_VERIFIED_CALENDAR_CLAIMS {
            return Err(Reason::CalendarUnproven);
        }
        for day in projection.days() {
            let date = day.session_date().as_naive_date();
            merge_calendar_suffix_claim(
                &mut facts,
                date,
                day.session_type(),
                start.as_naive_date(),
                end.as_naive_date(),
            )?;
        }
    }

    complete_calendar_suffix(facts, start.as_naive_date(), end.as_naive_date())
}

fn merge_calendar_suffix_claim(
    facts: &mut std::collections::BTreeMap<NaiveDate, DailySessionTypeV1>,
    date: NaiveDate,
    session_type: DailySessionTypeV1,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), Reason> {
    if date < start || date > end {
        return Ok(());
    }
    if facts
        .insert(date, session_type)
        .is_some_and(|previous| previous != session_type)
    {
        return Err(Reason::CalendarUnproven);
    }
    Ok(())
}

fn complete_calendar_suffix(
    mut facts: std::collections::BTreeMap<NaiveDate, DailySessionTypeV1>,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(NaiveDate, DailySessionTypeV1)>, Reason> {
    let span = end.signed_duration_since(start).num_days();
    if !(0..i64::try_from(MAX_CALENDAR_DAYS + 1).map_err(|_| Reason::CalendarUnproven)?)
        .contains(&span)
    {
        return Err(Reason::CalendarUnproven);
    }
    let count = usize::try_from(span + 1).map_err(|_| Reason::CalendarUnproven)?;
    let mut rows = Vec::with_capacity(count);
    for offset in 0..count {
        let date = start
            .checked_add_days(chrono::Days::new(
                u64::try_from(offset).map_err(|_| Reason::CalendarUnproven)?,
            ))
            .ok_or(Reason::CalendarUnproven)?;
        let session_type = facts.remove(&date).ok_or(Reason::CalendarUnproven)?;
        rows.push((date, session_type));
    }
    Ok(rows)
}

/// Clock data is not authority. Checked sessions can only be constructed below,
/// at the retained Source/SQL join. No wire/report-to-session conversion exists.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClockTriple {
    realtime_ns: i64,
    monotonic_ns: i64,
    boottime_ns: i64,
}

impl ClockTriple {
    pub(crate) fn from_value(value: &Value) -> Result<Self, Reason> {
        let object = value.as_object().ok_or(Reason::EvidenceTampered)?;
        if object.len() != 3 {
            return Err(Reason::EvidenceTampered);
        }
        let clock = |key: &str| {
            value[key]
                .as_i64()
                .filter(|v| *v >= 0)
                .ok_or(Reason::EvidenceTampered)
        };
        Ok(Self {
            realtime_ns: clock("realtime_ns")?,
            monotonic_ns: clock("monotonic_ns")?,
            boottime_ns: clock("boottime_ns")?,
        })
    }

    pub(crate) fn strictly_after(&self, earlier: &Self) -> bool {
        self.realtime_ns > earlier.realtime_ns
            && self.monotonic_ns > earlier.monotonic_ns
            && self.boottime_ns > earlier.boottime_ns
    }

    pub(crate) fn realtime_ns(&self) -> i64 {
        self.realtime_ns
    }
}

#[derive(Debug)]
pub(crate) struct CheckedArmOrigin {
    boot_id: Uuid,
    generation: Uuid,
    nonce: Uuid,
    invocation_id: Uuid,
    arm_intent_sha256: String,
    armed_sha256: String,
    arm_intent_observed: ClockTriple,
    armed_observed: ClockTriple,
}

impl CheckedArmOrigin {
    pub(crate) fn boot_id(&self) -> Uuid {
        self.boot_id
    }
    pub(crate) fn identities(&self) -> (Uuid, Uuid, Uuid) {
        (self.generation, self.nonce, self.invocation_id)
    }
    pub(crate) fn digests(&self) -> (&str, &str) {
        (&self.arm_intent_sha256, &self.armed_sha256)
    }
    pub(crate) fn observed(&self) -> (&ClockTriple, &ClockTriple) {
        (&self.arm_intent_observed, &self.armed_observed)
    }
}

#[derive(Debug)]
pub(crate) struct CheckedEnableSession {
    session_date: NaiveDate,
    run_id: Uuid,
    job_id: Uuid,
    attempt_id: Uuid,
    reservation_id: Uuid,
    slot_id: String,
    admission_sha256: String,
    source_event_sha256: String,
    root_event_sha256: String,
    cohort_sha256: String,
    arm: CheckedArmOrigin,
}

impl CheckedEnableSession {
    pub(crate) fn session_date(&self) -> NaiveDate {
        self.session_date
    }
    pub(crate) fn identities(&self) -> (Uuid, Uuid, Uuid, Uuid) {
        (
            self.run_id,
            self.job_id,
            self.attempt_id,
            self.reservation_id,
        )
    }
    pub(crate) fn slot_id(&self) -> &str {
        &self.slot_id
    }
    pub(crate) fn cohort_sha256(&self) -> &str {
        &self.cohort_sha256
    }
    pub(crate) fn arm(&self) -> &CheckedArmOrigin {
        &self.arm
    }

    // Only the original five SQL keys cross the SQL call. The genuine parent,
    // source descriptors and scope endpoints remain in the borrowed Session.
    pub(crate) fn sql_projection(&self) -> Value {
        serde_json::json!({
            "session_date": self.session_date.to_string(), "run_id": self.run_id,
            "admission_sha256": self.admission_sha256,
            "source_event_sha256": self.source_event_sha256,
            "root_event_sha256": self.root_event_sha256,
        })
    }
}

fn checked_source_uuid(value: &Value) -> Result<Uuid, Reason> {
    let text = value.as_str().ok_or(Reason::EvidenceTampered)?;
    let id = Uuid::parse_str(text).map_err(|_| Reason::EvidenceTampered)?;
    if id.is_nil() || id.to_string() != text {
        return Err(Reason::EvidenceTampered);
    }
    Ok(id)
}

fn checked_source_digest(value: &Value) -> Result<String, Reason> {
    let text = value.as_str().ok_or(Reason::EvidenceTampered)?;
    if text.len() != 71
        || !text.starts_with("sha256:")
        || !text[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Reason::EvidenceTampered);
    }
    Ok(text.to_owned())
}

struct DatabaseEvidence {
    pins: Value,
    source_identities: Vec<String>,
    run_identities: Vec<String>,
    original_identities: Vec<String>,
    native_enable_session: Option<CheckedEnableSession>,
    readback_sha256: String,
    admission_projection_sha256: String,
    current_rights_verified: bool,
    current_admission_verified: bool,
}

fn verify_database_readback<S: CheckerSourcePolicy>(
    source: &S,
    readback: &UntrustedPaperSessionReadbackV1,
    slot_id: &str,
    date: NaiveDate,
    cohort: &Value,
) -> Result<DatabaseEvidence, S::Error> {
    verify_database_readback_mode(source, readback, slot_id, date, cohort, true)
}

fn verify_database_admission_readback<S: CheckerSourcePolicy>(
    source: &S,
    readback: &UntrustedPaperSessionReadbackV1,
    slot_id: &str,
    date: NaiveDate,
    cohort: &Value,
) -> Result<DatabaseEvidence, S::Error> {
    verify_database_readback_mode(source, readback, slot_id, date, cohort, false)
}

fn verify_database_readback_mode<S: CheckerSourcePolicy>(
    source: &S,
    readback: &UntrustedPaperSessionReadbackV1,
    slot_id: &str,
    date: NaiveDate,
    cohort: &Value,
    require_run: bool,
) -> Result<DatabaseEvidence, S::Error> {
    let value = readback.as_untrusted_value();
    let first = source.policy_first()?;
    let reservation = first["reservation_id"]
        .as_str()
        .ok_or(Reason::EvidenceTampered)?;
    let root_event = source.policy_event("RESERVED", Reason::OriginUnproven)?;
    let root_event_sha256 = canonical_source_digest(root_event)?;
    let slot = &value["slot"];
    if slot["slot_id"] != slot_id
        || slot["session_date"] != date.to_string()
        || slot["owner_user_id"] != cohort["owner_user_id"]
        || slot["policy_sha256"] != cohort["policy_sha256"]
        || slot["activation_epoch"] != cohort["activation_epoch"]
        || slot["cohort_sha256"] != cohort["cohort_sha256"]
        || slot["cohort"] != *cohort
        || slot["reservation_id"] != reservation
        || slot["root_event_sha256"] != root_event_sha256
    {
        return Err(Reason::CohortChanged.into());
    }

    let finished = source.policy_event("SOURCE_FINISHED", Reason::EvidenceMissing)?;
    let finished_hash = canonical_source_digest(finished)?;
    let linked_admission = source.policy_event("ADMISSION_LINKED", Reason::EvidenceMissing)?;
    let admission = &value["admission"];
    let candidate_pins = &value["candidate_pins"];
    if admission.is_null() || candidate_pins.is_null() {
        return Err(Reason::InputNotAdmitted.into());
    }
    if admission["slot_id"] != slot_id
        || admission["source_event_sha256"] != finished_hash
        || admission["source_event_sha256"] != linked_admission["data"]["source_event_sha256"]
        || admission["original_set_sha256"] != finished["data"]["original_set_sha256"]
        || admission["original_set_sha256"] != linked_admission["data"]["original_set_sha256"]
        || !admission_link_matches(linked_admission, admission)
        || admission["admission_sha256"] != candidate_pins["admission_sha256"]
        || admission["admission_sha256"] != value["candidate_admission_sha256"]
    {
        return Err(Reason::InputPinMismatch.into());
    }

    let admission_projection_sha256 = stable_admission_projection_sha256(value)?;
    let mut run_identities = Vec::new();
    let mut native_enable_session = None;
    let readback_sha256 = if require_run {
        if value["pins"] != *candidate_pins {
            return Err(Reason::InputPinMismatch.into());
        }
        let run_event = source.policy_event("RUN_LINKED", Reason::EvidenceMissing)?;
        let run = &value["run"];
        if run.is_null()
            || run["slot_id"] != slot_id
            || run["run_id"] != value["candidate_run_id"]
            || run["job_id"] != value["job_id"]
            || run["attempt_id"] != value["attempt_id"]
            || run["run_id"] != run_event["data"]["run_id"]
            || run["job_id"] != run_event["data"]["job_id"]
            || run["attempt_id"] != run_event["data"]["attempt_id"]
            || value["terminal"] != "SUCCEEDED"
            || value["factor_snapshot_sha256"].is_null()
            || value["target_snapshot_sha256"].is_null()
        {
            return Err(Reason::SessionNotConsecutive.into());
        }
        let readback_sha256 =
            wire::digest(&wire::canonical(value).map_err(|_| Reason::EvidenceTampered)?);
        if run_event["data"]["readback_sha256"] != readback_sha256 {
            return Err(Reason::EvidenceTampered.into());
        }
        let attempts = value["attempts"]
            .as_array()
            .ok_or(Reason::EvidenceTampered)?;
        if attempts.is_empty() || attempts.len() > 3 {
            return Err(Reason::SessionNotConsecutive.into());
        }
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt["attempt_no"].as_u64() != Some((index + 1) as u64) {
                return Err(Reason::EvidenceTampered.into());
            }
            let outcome = attempt["outcome"]
                .as_str()
                .ok_or(Reason::EvidenceTampered)?;
            let last = index + 1 == attempts.len();
            if (last && (outcome != "SUCCEEDED" || attempt["id"] != value["attempt_id"]))
                || (!last && !matches!(outcome, "FAILED" | "ORPHANED"))
            {
                return Err(Reason::SessionNotConsecutive.into());
            }
        }
        run_identities = ["run_id", "job_id", "attempt_id"]
            .into_iter()
            .map(|key| {
                run[key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(Reason::EvidenceTampered)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A pure policy/Vec fixture has no scoped origin and cannot mint this
        // value. Only ScopedSource overrides policy_checked_arm, after checking
        // its actual retained owner and chain; source parent stays in that owner.
        if let Some(arm) = source.policy_checked_arm()? {
            if first["slot_id"] != slot_id
                || first["session"] != date.to_string()
                || first["cohort"] != *cohort
            {
                return Err(Reason::CohortChanged.into());
            }
            native_enable_session = Some(CheckedEnableSession {
                session_date: date,
                run_id: checked_source_uuid(&run["run_id"])?,
                job_id: checked_source_uuid(&run["job_id"])?,
                attempt_id: checked_source_uuid(&run["attempt_id"])?,
                reservation_id: checked_source_uuid(&slot["reservation_id"])?,
                slot_id: checked_source_digest(&slot["slot_id"])?,
                admission_sha256: checked_source_digest(&admission["admission_sha256"])?,
                source_event_sha256: finished_hash,
                root_event_sha256,
                cohort_sha256: checked_source_digest(&cohort["cohort_sha256"])?,
                arm,
            });
        }
        readback_sha256
    } else {
        wire::digest(&wire::canonical(value).map_err(|_| Reason::EvidenceTampered)?)
    };

    let originals = finished["data"]["originals"]
        .as_array()
        .ok_or(Reason::EvidenceTampered)?;
    let original_ids = originals
        .iter()
        .map(original_identity)
        .collect::<Result<Vec<_>, _>>()?;
    let raw = source.policy_event("RAW_SOURCE", Reason::OriginUnproven)?;
    let mut source_ids = vec![
        slot_id.to_owned(),
        reservation.to_owned(),
        first["invocation_id"]
            .as_str()
            .ok_or(Reason::EvidenceTampered)?
            .to_owned(),
        first["arm"]["generation"]
            .as_str()
            .ok_or(Reason::EvidenceTampered)?
            .to_owned(),
        first["arm"]["nonce"]
            .as_str()
            .ok_or(Reason::EvidenceTampered)?
            .to_owned(),
        raw["data"]["batch_id"]
            .as_str()
            .ok_or(Reason::EvidenceTampered)?
            .to_owned(),
        raw["data"]["calendar_batch_id"]
            .as_str()
            .ok_or(Reason::EvidenceTampered)?
            .to_owned(),
    ];
    for event in source
        .policy_events()?
        .iter()
        .filter(|event| event["kind"] == "CONTAINER")
    {
        source_ids.push(
            event["data"]["id"]
                .as_str()
                .ok_or(Reason::EvidenceTampered)?
                .to_owned(),
        );
    }
    if value["admitted_at"].is_null() || (require_run && value["finished_at"].is_null()) {
        return Err(Reason::SessionNotConsecutive.into());
    }
    Ok(DatabaseEvidence {
        pins: candidate_pins.clone(),
        source_identities: source_ids,
        run_identities,
        original_identities: original_ids,
        native_enable_session,
        readback_sha256,
        admission_projection_sha256,
        // A successful execution of the worker-only SQL reader proves current
        // rights and admission usability through paper_0066_authority and
        // daily_0060_admission_usable, respectively.
        current_rights_verified: true,
        current_admission_verified: true,
    })
}

fn stable_admission_projection_sha256(value: &Value) -> Result<String, Reason> {
    let projection = serde_json::json!({
        "schema": value["schema"],
        "slot": value["slot"],
        "admission": value["admission"],
        "candidate_admission_sha256": value["candidate_admission_sha256"],
        "candidate_pins": value["candidate_pins"],
        "admitted_at": value["admitted_at"],
    });
    if projection["schema"].is_null()
        || projection["slot"].is_null()
        || projection["admission"].is_null()
        || projection["candidate_admission_sha256"].is_null()
        || projection["candidate_pins"].is_null()
        || projection["admitted_at"].is_null()
    {
        return Err(Reason::EvidenceTampered);
    }
    Ok(wire::digest(
        &wire::canonical(&projection).map_err(|_| Reason::EvidenceTampered)?,
    ))
}

fn sql_reason(error: &sqlx::Error) -> Reason {
    match error
        .as_database_error()
        .and_then(|database| database.code())
    {
        Some(code) if code.as_ref() == "42501" => Reason::RightsUnavailable,
        Some(code) if code.as_ref() == "23514" => Reason::EvidenceMissing,
        _ => Reason::ReaderIncompatible,
    }
}

fn admission_link_matches(linked_admission: &Value, database_admission: &Value) -> bool {
    linked_admission["data"]["admission_sha256"]
        .as_str()
        .zip(database_admission["admission_sha256"].as_str())
        .is_some_and(|(linked, database)| linked == database)
}

fn validate_source_admission_state(events: &[Value], require_run: bool) -> Result<(), Reason> {
    fn kind(event: &Value) -> &str {
        event["kind"].as_str().unwrap_or_default()
    }
    let admission_index = events
        .iter()
        .position(|event| kind(event) == "ADMISSION_LINKED")
        .ok_or(Reason::EvidenceMissing)?;
    if require_run {
        if kind(events.last().ok_or(Reason::OriginUnproven)?) != "LINKED" {
            return Err(Reason::OriginUnproven);
        }
        return Ok(());
    }

    let run_after_admission = events
        .iter()
        .enumerate()
        .any(|(index, event)| index > admission_index && kind(event) == "RUN_LINKED");
    let failures = events
        .iter()
        .enumerate()
        .filter(|(_, event)| kind(event) == "FAILED")
        .collect::<Vec<_>>();
    if let Some((failed_index, failed)) = failures.first().copied() {
        if failures.len() != 1
            || failed_index <= admission_index
            || events.last().map(kind) != Some("FAILED")
            || failed["data"]["code"] != "LINK_PENDING"
            || run_after_admission
        {
            return Err(Reason::OriginUnproven);
        }
        return Ok(());
    }

    if !matches!(
        kind(events.last().ok_or(Reason::OriginUnproven)?),
        "ADMISSION_LINKED" | "LINKED" | "RUN_LINKED"
    ) {
        return Err(Reason::OriginUnproven);
    }
    Ok(())
}

fn verify_daily_raw<S: CheckerSourcePolicy, R: CheckerRawPolicy<Error = S::Error>>(
    source: &S,
    session: &DailySessionEvidenceV1Wire,
    snapshots: &mut R,
    date: TradingDate,
) -> Result<VerifiedDailyRawEvidence, S::Error> {
    let finished = source.policy_event("SOURCE_FINISHED", Reason::EvidenceMissing)?;
    let originals = finished["data"]["originals"]
        .as_array()
        .ok_or(Reason::EvidenceTampered)?;
    let mut files = session
        .bars
        .iter()
        .chain(&session.reference)
        .collect::<Vec<_>>();
    files.extend(
        session
            .calendar
            .iter()
            .filter(|calendar| calendar.civil_date == date)
            .map(|calendar| &calendar.source_file),
    );
    if files.len() != 23 {
        return Err(Reason::InputNotAdmitted.into());
    }

    let mut actual = Vec::with_capacity(files.len());
    let mut references = Vec::with_capacity(files.len());
    for evidence in files {
        let reference = SourceFileReference {
            provider: evidence.provider.clone(),
            market: evidence.market.clone(),
            batch_id: evidence.batch_id,
            file_name: evidence.file_name.clone(),
            content_hash: evidence.wire_sha256.clone(),
            retrieved_at: evidence.retrieved_at.as_timestamp(),
        };
        if !snapshots.matches_original(&reference, evidence)? {
            return Err(Reason::EvidenceTampered.into());
        }
        references.push(reference);
        let complete = serde_json::to_value(evidence).map_err(|_| Reason::EvidenceTampered)?;
        let original = complete["copied_from"]
            .as_array()
            .and_then(|chain| chain.last())
            .unwrap_or(&complete);
        let mut item = serde_json::Map::new();
        for key in [
            "provider",
            "market",
            "batch_id",
            "batch_date",
            "file_name",
            "wire_sha256",
            "size_bytes",
            "batch_json_sha256",
            "manifest_line_sha256",
            "retrieved_at",
        ] {
            item.insert(key.to_owned(), original[key].clone());
        }
        let identity = original_identity(&Value::Object(item.clone()))?;
        if let Some(expected) = originals
            .iter()
            .find(|expected| original_identity(expected).ok().as_deref() == Some(identity.as_str()))
        {
            let actual_time = item["retrieved_at"]
                .as_str()
                .ok_or(Reason::EvidenceTampered)?;
            let expected_time = expected["retrieved_at"]
                .as_str()
                .ok_or(Reason::EvidenceTampered)?;
            if DateTime::parse_from_rfc3339(actual_time).ok()
                != DateTime::parse_from_rfc3339(expected_time).ok()
            {
                return Err(Reason::EvidenceTampered.into());
            }
            item.insert("retrieved_at".to_owned(), expected["retrieved_at"].clone());
        } else {
            return Err(Reason::EvidenceTampered.into());
        }
        actual.push(Value::Object(item));
    }
    actual.sort_by_key(original_sort_key);
    if actual != *originals
        || wire::original_set_hash(&actual).map_err(|_| Reason::EvidenceTampered)?
            != string(&finished["data"], "original_set_sha256")?
    {
        return Err(Reason::EvidenceTampered.into());
    }

    for action in &session.actions {
        for evidence in &action.pages {
            let reference = SourceFileReference {
                provider: evidence.provider.clone(),
                market: evidence.market.clone(),
                batch_id: evidence.batch_id,
                file_name: evidence.file_name.clone(),
                content_hash: evidence.wire_sha256.clone(),
                retrieved_at: evidence.retrieved_at.as_timestamp(),
            };
            if !snapshots.matches_original(&reference, evidence)? {
                return Err(Reason::EvidenceTampered.into());
            }
            references.push(reference);
        }
    }
    let action_scopes = session
        .actions
        .iter()
        .map(action_scope_evidence)
        .collect::<Result<Vec<_>, _>>()?;
    if !action_source_matches(&finished["data"]["action"], &action_scopes) {
        return Err(Reason::EvidenceTampered.into());
    }
    let original_identities = originals
        .iter()
        .map(original_identity)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(VerifiedDailyRawEvidence {
        original_identities,
        references,
    })
}

struct VerifiedDailyRawEvidence {
    original_identities: Vec<String>,
    references: Vec<SourceFileReference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActionScopeEvidence {
    range_start: String,
    range_end: String,
    page_batch_ids: Vec<Value>,
}

fn action_scope_evidence(action: &DailyActionChainV1Wire) -> Result<ActionScopeEvidence, Reason> {
    let page_batch_ids = action
        .pages
        .iter()
        .map(|page| serde_json::to_value(page.batch_id).map_err(|_| Reason::EvidenceTampered))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ActionScopeEvidence {
        range_start: action.range_start.to_iso(),
        range_end: action.range_end.to_iso(),
        page_batch_ids,
    })
}

fn action_source_matches(source_action: &Value, actions: &[ActionScopeEvidence]) -> bool {
    let Some(batch_id) = source_action["batch_id"].as_str() else {
        return source_action["batch_id"].is_null();
    };
    let Some(start) = source_action["start"].as_str() else {
        return false;
    };
    let Some(end) = source_action["end"].as_str() else {
        return false;
    };
    actions.iter().any(|action| {
        !action.page_batch_ids.is_empty()
            && action.range_start == start
            && action.range_end == end
            && action
                .page_batch_ids
                .iter()
                .all(|page_batch_id| page_batch_id.as_str() == Some(batch_id))
    })
}

fn original_identity(value: &Value) -> Result<String, Reason> {
    Ok(["provider", "market", "batch_id", "file_name"]
        .into_iter()
        .map(|key| {
            value[key]
                .as_str()
                .ok_or(Reason::EvidenceTampered)
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("|"))
}

fn original_sort_key(value: &Value) -> [String; 5] {
    ["provider", "market", "batch_id", "file_name", "wire_sha256"]
        .map(|key| value[key].as_str().unwrap_or_default().to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionMode {
    Scheduled,
    #[cfg(test)]
    Manual,
    #[cfg(test)]
    CatchUp,
    #[cfg(test)]
    Unknown,
}

#[derive(Clone, Debug)]
struct VerifiedSessionFacts {
    date: NaiveDate,
    cohort: Value,
    source_identities: Vec<String>,
    run_identities: Vec<String>,
    original_identities: Vec<String>,
    mode: SessionMode,
    terminal_succeeded: bool,
    rights_active: bool,
    rights_sufficient: bool,
    source_revoked: bool,
    run_linked: bool,
    input_admitted: bool,
    pins_match: bool,
    price_available: bool,
    replayed: bool,
    revalidated: bool,
    daily_hashes: [String; 4],
}

fn evaluate_verified_sessions(
    sessions: &[Result<VerifiedSessionFacts, Reason>],
    expected_cohort: &Value,
    expected_dates: &[NaiveDate],
    eligible_date: NaiveDate,
    evaluated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> PaperEligibilityReport {
    let mut reasons = Vec::new();
    if sessions.len() != MAX_SESSIONS || expected_dates.len() != MAX_SESSIONS {
        reasons.push(Reason::CalendarUnproven);
    }
    if expected_dates.windows(2).any(|pair| pair[0] >= pair[1])
        || expected_dates.iter().any(|date| *date > eligible_date)
    {
        reasons.push(Reason::SessionNotConsecutive);
    }
    if expires_at <= evaluated_at {
        reasons.push(Reason::EligibilityExpired);
    }
    let mut all_ids = BTreeSet::new();
    for (index, candidate) in sessions.iter().enumerate() {
        let session = match candidate {
            Ok(session) => session,
            Err(reason) => {
                reasons.push(*reason);
                continue;
            }
        };
        if expected_dates.get(index) != Some(&session.date) {
            reasons.push(Reason::SessionNotConsecutive);
        }
        if session.date > eligible_date {
            reasons.push(Reason::FutureSession);
        }
        if session.cohort["owner_user_id"] != expected_cohort["owner_user_id"] {
            reasons.push(Reason::OwnerScopeMismatch);
        }
        if session.cohort["strategy_config_id"] != expected_cohort["strategy_config_id"]
            || session.cohort["config_epoch"] != expected_cohort["config_epoch"]
            || session.cohort["strategy_config_sha256"] != expected_cohort["strategy_config_sha256"]
        {
            reasons.push(Reason::StrategyBindingMismatch);
        }
        if session.cohort != *expected_cohort {
            reasons.push(Reason::CohortChanged);
        }
        match session.mode {
            SessionMode::Scheduled => {}
            #[cfg(test)]
            SessionMode::Manual | SessionMode::CatchUp | SessionMode::Unknown => {
                reasons.push(Reason::ManualIntervention);
            }
        }
        if !session.terminal_succeeded || !session.run_linked {
            reasons.push(Reason::SessionNotConsecutive);
        }
        if session.replayed {
            reasons.push(Reason::ReplayDetected);
        }
        if !session.revalidated {
            reasons.push(Reason::EvidenceTampered);
        }
        if !session.rights_active || !session.rights_sufficient {
            reasons.push(Reason::RightsUnavailable);
        }
        if session.source_revoked {
            reasons.push(Reason::SourceRevoked);
        }
        if !session.input_admitted {
            reasons.push(Reason::InputNotAdmitted);
        }
        if !session.pins_match {
            reasons.push(Reason::InputPinMismatch);
        }
        if session
            .daily_hashes
            .iter()
            .any(|hash| ContentHash::parse(hash).is_err())
            || expected_cohort["policy_sha256"].as_str() != Some(session.daily_hashes[3].as_str())
        {
            reasons.push(Reason::InputPinMismatch);
        }
        if !session.price_available {
            reasons.push(Reason::PriceNotAvailable);
        }
        for identity in session
            .source_identities
            .iter()
            .chain(&session.run_identities)
            .chain(&session.original_identities)
        {
            if !all_ids.insert(identity.clone()) {
                reasons.push(Reason::DuplicateIdentity);
            }
        }
    }
    let report_sessions = expected_dates.iter().map(ToString::to_string).collect();
    PaperEligibilityReport::new(
        if reasons.is_empty() {
            PaperEligibilityDecision::Eligible
        } else {
            PaperEligibilityDecision::Ineligible
        },
        reasons,
        report_sessions,
        evaluated_at,
        expires_at,
    )
}

fn eligibility_expiry(now: DateTime<Utc>, eligible_date: NaiveDate) -> Option<DateTime<Utc>> {
    let next_date = eligible_date.checked_add_days(chrono::Days::new(1))?;
    let next_cutoff = Utc
        .with_ymd_and_hms(
            next_date.year(),
            next_date.month(),
            next_date.day(),
            7,
            30,
            0,
        )
        .single()?;
    Some(std::cmp::min(
        now + chrono::Duration::seconds(ELIGIBILITY_TTL_SECONDS),
        next_cutoff,
    ))
}

fn latest_three_trading_sessions(
    complete_rows: &[(NaiveDate, DailySessionTypeV1)],
    eligible_date: NaiveDate,
) -> Result<[NaiveDate; MAX_SESSIONS], Reason> {
    let Some((first, _)) = complete_rows.first() else {
        return Err(Reason::CalendarUnproven);
    };
    if complete_rows.last().map(|(date, _)| *date) != Some(eligible_date) {
        return Err(Reason::CalendarUnproven);
    }
    for (index, (date, _)) in complete_rows.iter().enumerate() {
        let offset = u64::try_from(index).map_err(|_| Reason::CalendarUnproven)?;
        if *date
            != first
                .checked_add_days(chrono::Days::new(offset))
                .ok_or(Reason::CalendarUnproven)?
            || *date > eligible_date
        {
            return Err(Reason::CalendarUnproven);
        }
    }
    let trading = complete_rows
        .iter()
        .filter_map(|(date, kind)| (*kind == DailySessionTypeV1::Trading).then_some(*date))
        .collect::<Vec<_>>();
    if trading.len() < MAX_SESSIONS {
        return Err(Reason::CalendarUnproven);
    }
    trading[trading.len() - MAX_SESSIONS..]
        .try_into()
        .map_err(|_| Reason::CalendarUnproven)
}

fn parse_scan_limits(value: &Value) -> Result<ScanLimitValues, Reason> {
    let size = |key: &str| -> Result<usize, Reason> {
        usize::try_from(value[key].as_u64().ok_or(Reason::ReaderIncompatible)?)
            .map_err(|_| Reason::ReaderIncompatible)
    };
    Ok(ScanLimitValues {
        max_manifest_bytes: size("max_manifest_bytes")?,
        max_manifest_records: size("max_manifest_records")?,
        max_batches: size("max_batches")?,
        max_files_per_batch: size("max_files_per_batch")?,
        max_files: size("max_files")?,
        max_batch_metadata_bytes: size("max_batch_metadata_bytes")?,
        max_source_file_bytes: size("max_source_file_bytes")?,
        max_retained_bytes: size("max_retained_bytes")?,
        max_open_descriptors: size("max_open_descriptors")?,
        max_data_root_bytes: size("max_data_root_bytes")?,
        max_data_root_components: size("max_data_root_components")?,
        max_metadata_json_depth: size("max_metadata_json_depth")?,
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, Reason> {
    value[key].as_str().ok_or(Reason::ReaderIncompatible)
}

fn daily_session_for_date(
    manifest: &market_data::daily_price_only::DailyPriceManifestV1Wire,
    date: TradingDate,
) -> Option<&DailySessionEvidenceV1Wire> {
    manifest
        .daily_sources
        .iter()
        .find(|session| session.session_date == date)
}

fn target_calendar_binding(
    session: &DailySessionEvidenceV1Wire,
    date: TradingDate,
) -> Option<&DailyCalendarBindingV1Wire> {
    session.calendar.iter().find(|calendar| {
        calendar.civil_date == date && calendar.session_type == DailySessionTypeV1::Trading
    })
}

fn canonical_source_digest(event: &Value) -> Result<String, Reason> {
    let bytes = wire::canonical(event).map_err(|_| Reason::EvidenceTampered)?;
    Ok(wire::digest(&bytes))
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use serde_json::{Value, json};

    use super::{
        ActionScopeEvidence, DAILY_ADMITTED_ALIAS, DAILY_CONFIG_ALIAS, DAILY_HISTORICAL_ALIAS,
        DAILY_POLICY_ALIAS, DAILY_RAW_ALIAS, EvaluationClock, MAX_SESSIONS, Reason, SessionMode,
        VerifiedSessionFacts, action_source_matches, admission_link_matches,
        authoritative_binding_from_snapshot, complete_calendar_suffix, evaluate_verified_sessions,
        latest_three_trading_sessions, latest_trading_candidates, merge_calendar_suffix_claim,
        selected_entitlement_batches, stable_admission_projection_sha256,
        validate_daily_alias_mapping, validate_source_admission_state,
    };
    use crate::owner_beta::paper::{
        contract::{PaperEligibilityDecision, PaperEligibilityReport, UntrustedPaperContractV1},
        source_evidence::{self as wire, UntrustedCurrentEnvelope},
    };
    use market_data::daily_price_only::DailySessionTypeV1;

    // SYNTHETIC: evaluator-only facts; these private constructors cannot
    // create source, Raw, Daily, database, or Paper authority.
    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("synthetic date")
    }

    fn hash(digit: char) -> String {
        format!("sha256:{}", digit.to_string().repeat(64))
    }

    // Untrusted wire/policy inputs only: these tests invoke the complete DB
    // validator, not a checked Source constructor or a financial capability.
    struct DatabasePolicy(Vec<Value>);
    #[test]
    fn untrusted_policy_and_report_json_cannot_mint_native_arm_origin() {
        let policy = DatabasePolicy(vec![json!({"kind":"ARMED","verified":true})]);
        assert!(
            super::CheckerSourcePolicy::policy_checked_arm(&policy)
                .unwrap()
                .is_none()
        );
        // This proves the mock/policy path has no constructor. Genuine Source
        // origin requires the retained checked reader in hosted integration.
    }

    #[test]
    fn native_arm_clock_causality_requires_all_three_strict_comparisons() {
        let anchor = super::ClockTriple::from_value(&json!({
            "realtime_ns":100,"monotonic_ns":200,"boottime_ns":300}))
        .unwrap();
        let post = json!({"realtime_ns":101,"monotonic_ns":201,"boottime_ns":301});
        assert!(
            super::ClockTriple::from_value(&post)
                .unwrap()
                .strictly_after(&anchor)
        );
        for (key, reset) in [
            ("realtime_ns", 100),
            ("monotonic_ns", 200),
            ("boottime_ns", 300),
        ] {
            for tick in [reset, reset - 1] {
                let mut bad = post.clone();
                bad[key] = json!(tick);
                assert!(
                    !super::ClockTriple::from_value(&bad)
                        .unwrap()
                        .strictly_after(&anchor),
                    "{key}"
                );
            }
        }
        // DB creation/link times do not enter this comparator and cannot bless
        // a pre-anchor ARM. This is data validation, not a genuine Source test.
    }
    impl super::CheckerSourcePolicy for DatabasePolicy {
        type Error = Reason;

        fn policy_first(&self) -> Result<&Value, Reason> {
            self.0.first().ok_or(Reason::EvidenceMissing)
        }

        fn policy_event(&self, kind: &str, reason: Reason) -> Result<&Value, Reason> {
            self.0
                .iter()
                .find(|event| event["kind"] == kind)
                .ok_or(reason)
        }

        fn policy_events(&self) -> Result<&[Value], Reason> {
            Ok(&self.0)
        }
    }

    fn admission_database_wire() -> (DatabasePolicy, Value, Value) {
        let cohort = cohort();
        let reservation = "00000000-0000-4000-8000-000000000001";
        let reserved = json!({
            "kind": "RESERVED", "reservation_id": reservation,
            "invocation_id": "00000000-0000-4000-8000-000000000002",
            "arm": {
                "generation": "00000000-0000-4000-8000-000000000003",
                "nonce": "00000000-0000-4000-8000-000000000004"
            }
        });
        let finished = json!({ "kind": "SOURCE_FINISHED", "data": {
            "original_set_sha256": hash('a'),
            "originals": [{ "provider": "kis", "market": "KR",
                "batch_id": "00000000-0000-4000-8000-000000000005",
                "file_name": "bars.json" }]
        }});
        let finished_hash = super::canonical_source_digest(&finished).unwrap();
        let pins = json!({ "admission_sha256": hash('b'), "content_sha256": hash('c'),
            "manifest_sha256": hash('d'), "policy_sha256": cohort["policy_sha256"] });
        let slot_id = hash('e');
        let admitted = json!({ "kind": "ADMISSION_LINKED", "data": {
            "source_event_sha256": finished_hash, "original_set_sha256": hash('a'),
            "admission_sha256": pins["admission_sha256"]
        }});
        let value = json!({
            "schema": "owner-beta-paper-db-v1",
            "slot": { "slot_id": slot_id, "reservation_id": reservation,
                "root_event_sha256": super::canonical_source_digest(&reserved).unwrap(),
                "owner_user_id": cohort["owner_user_id"], "policy_sha256": cohort["policy_sha256"],
                "activation_epoch": cohort["activation_epoch"], "session_date": "2026-11-05",
                "cohort_sha256": cohort["cohort_sha256"], "cohort": cohort,
                "reserved_at": "2026-11-05T07:30:00Z" },
            "admission": { "slot_id": slot_id, "source_event_sha256": finished_hash,
                "original_set_sha256": hash('a'), "admission_sha256": pins["admission_sha256"],
                "linked_at": "2026-11-05T07:31:00Z" },
            "candidate_admission_sha256": pins["admission_sha256"], "candidate_pins": pins,
            "admitted_at": "2026-11-05T07:31:00Z", "finished_at": null,
            "run": null, "candidate_run_id": null, "terminal": null,
            "job_id": null, "attempt_id": null, "attempts": null, "pins": null,
            "factor_snapshot_sha256": null, "target_snapshot_sha256": null
        });
        (
            DatabasePolicy(vec![
                reserved,
                json!({ "kind": "RAW_SOURCE", "data": {
                    "batch_id": "00000000-0000-4000-8000-000000000005",
                    "calendar_batch_id": "00000000-0000-4000-8000-000000000006"
                }}),
                finished,
                admitted,
            ]),
            value,
            cohort,
        )
    }

    #[test]
    fn complete_e_admission_validator_accepts_no_run_or_pending_attempt_without_finish() {
        let (source, no_run, cohort) = admission_database_wire();
        let mut pending = no_run.clone();
        pending["candidate_run_id"] = json!("00000000-0000-4000-8000-000000000007");
        pending["job_id"] = json!("00000000-0000-4000-8000-000000000008");
        pending["attempt_id"] = json!("00000000-0000-4000-8000-000000000009");
        pending["terminal"] = json!("RUNNING");
        pending["pins"] = pending["candidate_pins"].clone();
        pending["attempts"] = json!([{ "id": pending["attempt_id"], "attempt_no": 1,
            "outcome": "RUNNING", "started_at": "2026-11-05T07:32:00Z", "finished_at": null }]);
        let mut projections = Vec::new();
        for value in [no_run, pending] {
            assert!(value["finished_at"].is_null());
            let readback = super::UntrustedPaperSessionReadbackV1::from_value(value).unwrap();
            let validated = super::verify_database_admission_readback(
                &source,
                &readback,
                &hash('e'),
                date("2026-11-05"),
                &cohort,
            )
            .unwrap_or_else(|reason| {
                panic!("admitted E must not depend on recommendation finish: {reason:?}")
            });
            assert!(validated.run_identities.is_empty());
            assert_eq!(
                validated.pins,
                readback.as_untrusted_value()["candidate_pins"]
            );
            projections.push(validated.admission_projection_sha256);
        }
        assert_eq!(projections[0], projections[1]);
    }

    #[test]
    fn complete_t_validator_still_requires_finish_and_e_still_requires_admission() {
        let (mut source, mut value, cohort) = admission_database_wire();
        value["candidate_run_id"] = json!("00000000-0000-4000-8000-000000000007");
        value["job_id"] = json!("00000000-0000-4000-8000-000000000008");
        value["attempt_id"] = json!("00000000-0000-4000-8000-000000000009");
        value["terminal"] = json!("SUCCEEDED");
        value["pins"] = value["candidate_pins"].clone();
        value["factor_snapshot_sha256"] = json!(hash('1'));
        value["target_snapshot_sha256"] = json!(hash('2'));
        value["finished_at"] = json!("2026-11-05T07:33:00Z");
        value["attempts"] = json!([{ "id": value["attempt_id"], "attempt_no": 1,
            "outcome": "SUCCEEDED", "started_at": "2026-11-05T07:32:00Z",
            "finished_at": value["finished_at"] }]);
        value["run"] = json!({ "slot_id": hash('e'), "run_id": value["candidate_run_id"],
            "job_id": value["job_id"], "attempt_id": value["attempt_id"],
            "linked_at": "2026-11-05T07:33:00Z" });
        source.0.push(json!({ "kind": "RUN_LINKED", "data": {
            "run_id": value["candidate_run_id"], "job_id": value["job_id"],
            "attempt_id": value["attempt_id"],
            "readback_sha256": super::canonical_source_digest(&value).unwrap()
        }}));
        let run_event = source.0.len() - 1;
        source.0.push(json!({ "kind": "LINKED" }));
        let good = super::UntrustedPaperSessionReadbackV1::from_value(value.clone()).unwrap();
        assert!(
            super::verify_database_readback(
                &source,
                &good,
                &hash('e'),
                date("2026-11-05"),
                &cohort
            )
            .is_ok()
        );
        value["finished_at"] = Value::Null;
        source.0[run_event]["data"]["readback_sha256"] =
            json!(super::canonical_source_digest(&value).unwrap());
        let missing_finish =
            super::UntrustedPaperSessionReadbackV1::from_value(value.clone()).unwrap();
        assert!(matches!(
            super::verify_database_readback(
                &source,
                &missing_finish,
                &hash('e'),
                date("2026-11-05"),
                &cohort
            ),
            Err(Reason::SessionNotConsecutive)
        ));
        value["admitted_at"] = Value::Null;
        let missing_admission = super::UntrustedPaperSessionReadbackV1::from_value(value).unwrap();
        assert!(
            super::verify_database_admission_readback(
                &source,
                &missing_admission,
                &hash('e'),
                date("2026-11-05"),
                &cohort
            )
            .is_err()
        );
    }

    #[test]
    fn stable_execution_admission_ignores_later_recommendation_progress() {
        let base = json!({
            "schema": "owner-beta-paper-session-evidence-v1",
            "slot": { "slot_id": "slot", "session_date": "2026-11-05" },
            "admission": { "admission_sha256": hash('1') },
            "candidate_admission_sha256": hash('1'),
            "candidate_pins": { "admission_sha256": hash('1'), "content_sha256": hash('2') },
            "admitted_at": "2026-11-05T00:00:00Z",
            "terminal": "RUNNING",
            "attempts": []
        });
        let mut later = base.clone();
        later["terminal"] = json!("FAILED");
        later["attempts"] = json!([{ "attempt_no": 1, "outcome": "FAILED" }]);

        assert_eq!(
            stable_admission_projection_sha256(&base).unwrap(),
            stable_admission_projection_sha256(&later).unwrap()
        );
        later["candidate_pins"]["content_sha256"] = json!(hash('3'));
        assert_ne!(
            stable_admission_projection_sha256(&base).unwrap(),
            stable_admission_projection_sha256(&later).unwrap()
        );
    }

    #[test]
    fn execution_admission_accepts_real_prefix_and_only_link_pending_failure() {
        let admitted = vec![
            json!({ "kind": "RESERVED" }),
            json!({ "kind": "SOURCE_FINISHED" }),
            json!({ "kind": "ADMISSION_LINKED" }),
        ];
        assert!(validate_source_admission_state(&admitted, false).is_ok());
        assert!(validate_source_admission_state(&admitted, true).is_err());

        let mut pending = admitted.clone();
        pending.push(json!({ "kind": "FAILED", "data": { "code": "LINK_PENDING" } }));
        assert!(validate_source_admission_state(&pending, false).is_ok());

        for code in ["SOURCE_INVALID", "INTERRUPTED", "CLOCK_INVALID"] {
            let mut failed = admitted.clone();
            failed.push(json!({ "kind": "FAILED", "data": { "code": code } }));
            assert!(validate_source_admission_state(&failed, false).is_err());
        }
        let mut nonterminal_failure = admitted.clone();
        nonterminal_failure.push(json!({ "kind": "FAILED", "data": { "code": "LINK_PENDING" } }));
        nonterminal_failure.push(json!({ "kind": "LINKED" }));
        assert!(validate_source_admission_state(&nonterminal_failure, false).is_err());
        let mut repeated_failure = admitted.clone();
        repeated_failure.push(json!({ "kind": "FAILED", "data": { "code": "LINK_PENDING" } }));
        repeated_failure.push(json!({ "kind": "FAILED", "data": { "code": "LINK_PENDING" } }));
        assert!(validate_source_admission_state(&repeated_failure, false).is_err());
        let mut run_then_pending = admitted;
        run_then_pending.push(json!({ "kind": "RUN_LINKED" }));
        run_then_pending.push(json!({ "kind": "FAILED", "data": { "code": "LINK_PENDING" } }));
        assert!(validate_source_admission_state(&run_then_pending, false).is_err());
    }

    fn cohort() -> Value {
        json!({
            "owner_user_id": "7b1f9ad8-29f1-4bf5-9ce6-980e8db0905b",
            "policy_sha256": hash('1'),
            "activation_epoch": 4,
            "strategy_config_id": "c745e1f1-19d3-4de8-9c98-18c4ed7f80a5",
            "config_epoch": 5,
            "strategy_config_sha256": hash('2'),
            "entitlement_reference": "fixture-entitlement-ref",
            "entitlement_sha256": hash('3'),
            "cohort_sha256": hash('4'),
            "release_manifest_sha256": hash('5'),
            "research_image_id": hash('6'),
            "helper_image_id": hash('7'),
            "semantic_config_sha256": hash('8'),
            "calendar_sha256": hash('9'),
            "verifier_revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    fn verified(date: NaiveDate, cohort: &Value, n: usize) -> VerifiedSessionFacts {
        let day = char::from_digit((n + 1) as u32, 16).expect("hex digit");
        VerifiedSessionFacts {
            date,
            cohort: cohort.clone(),
            source_identities: vec![format!("source-{n}")],
            run_identities: vec![format!("run-{n}")],
            original_identities: vec![format!("original-{n}")],
            mode: SessionMode::Scheduled,
            terminal_succeeded: true,
            rights_active: true,
            rights_sufficient: true,
            source_revoked: false,
            run_linked: true,
            input_admitted: true,
            pins_match: true,
            price_available: true,
            replayed: false,
            revalidated: true,
            // Date-specific input pins are expected to differ. Policy stays
            // common and is the only hash pinned to the cohort.
            daily_hashes: [
                hash(day),
                hash(char::from_digit((n + 4) as u32, 16).unwrap()),
                hash(char::from_digit((n + 7) as u32, 16).unwrap()),
                hash('1'),
            ],
        }
    }

    fn sessions(cohort: &Value) -> Vec<Result<VerifiedSessionFacts, Reason>> {
        ["2026-10-30", "2026-11-02", "2026-11-04"]
            .into_iter()
            .enumerate()
            .map(|(index, value)| Ok(verified(date(value), cohort, index)))
            .collect()
    }

    fn report(
        facts: &[Result<VerifiedSessionFacts, Reason>],
        expected: &[NaiveDate],
        cohort: &Value,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PaperEligibilityReport {
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        evaluate_verified_sessions(facts, cohort, expected, date("2026-11-04"), now, expires_at)
    }

    fn reason_codes(report: &PaperEligibilityReport) -> Vec<&'static str> {
        report.reason_codes().collect()
    }

    fn expected_dates() -> [NaiveDate; MAX_SESSIONS] {
        [date("2026-10-30"), date("2026-11-02"), date("2026-11-04")]
    }

    fn synthetic_current_and_candidate() -> (UntrustedCurrentEnvelope, UntrustedPaperContractV1) {
        const CURRENT: &[u8] = include_bytes!(
            "../../../../../tests/fixtures/owner_beta_paper_current_binding/valid-current.json"
        );
        let current = wire::decode_current_envelope(CURRENT).expect("synthetic CURRENT fixture");
        let binding = wire::canonical(&current.value()["binding"]).expect("binding canonical");
        let candidate = UntrustedPaperContractV1::decode(&binding).expect("untrusted candidate");
        (current, candidate)
    }

    fn synthetic_daily_settings() -> Value {
        json!({
            "owner": "00000000-0000-4000-8000-000000000001",
            "policy_sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "policy_path": DAILY_POLICY_ALIAS,
            "historical_root": DAILY_HISTORICAL_ALIAS,
            "raw_root": DAILY_RAW_ALIAS,
            "admitted_root": DAILY_ADMITTED_ALIAS,
            "raw_owner_uid": 10001,
            "entitlement_reference": "fixture-personal",
            "entitlement_sha256": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "code_commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    fn utc(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .expect("synthetic timestamp")
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn synthetic_current_binding_is_compared_as_the_complete_host_selector() {
        let (current, candidate) = synthetic_current_and_candidate();
        let authoritative = authoritative_binding_from_snapshot(&current, &candidate)
            .expect("matching complete host binding");
        assert_eq!(&authoritative, candidate.as_untrusted_value());
        assert_eq!(authoritative["data_root"], "/synthetic/data");
        assert_eq!(
            authoritative["daily_config_file"],
            "/synthetic/releases/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/.owner-beta-daily/config.json"
        );

        let mut old_alias_candidate = candidate.as_untrusted_value().clone();
        old_alias_candidate["daily_config_file"] = json!(DAILY_CONFIG_ALIAS);
        let bytes = wire::canonical(&old_alias_candidate).expect("canonical changed selector");
        let old_alias_candidate =
            UntrustedPaperContractV1::decode(&bytes).expect("shape-valid changed selector");
        assert!(authoritative_binding_from_snapshot(&current, &old_alias_candidate).is_err());

        let mut old_data_alias_candidate = candidate.as_untrusted_value().clone();
        old_data_alias_candidate["data_root"] = json!("/data");
        let bytes = wire::canonical(&old_data_alias_candidate).expect("canonical changed selector");
        let old_data_alias_candidate =
            UntrustedPaperContractV1::decode(&bytes).expect("shape-valid changed selector");
        assert!(authoritative_binding_from_snapshot(&current, &old_data_alias_candidate).is_err());
    }

    #[test]
    fn synthetic_daily_alias_mapping_keeps_host_selectors_distinct_from_mount_aliases() {
        let (current, _) = synthetic_current_and_candidate();
        let binding = &current.value()["binding"];
        let settings = synthetic_daily_settings();
        let pin = binding["daily_config_sha256"]
            .as_str()
            .expect("current pin");
        assert!(
            validate_daily_alias_mapping(
                binding,
                &settings,
                DAILY_CONFIG_ALIAS,
                pin,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            )
            .is_ok()
        );

        let mut wrong_config_alias = settings.clone();
        wrong_config_alias["policy_path"] = json!("/synthetic/releases/policy.json");
        assert_eq!(
            validate_daily_alias_mapping(
                binding,
                &wrong_config_alias,
                DAILY_CONFIG_ALIAS,
                pin,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            Err(Reason::CohortChanged)
        );

        let mut wrong_raw_alias = settings.clone();
        wrong_raw_alias["raw_root"] = json!("/synthetic/data/raw");
        assert_eq!(
            validate_daily_alias_mapping(
                binding,
                &wrong_raw_alias,
                DAILY_CONFIG_ALIAS,
                pin,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            Err(Reason::CohortChanged)
        );

        assert_eq!(
            validate_daily_alias_mapping(
                binding,
                &settings,
                "/synthetic/releases/config.json",
                pin,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            Err(Reason::ReaderIncompatible)
        );
        assert_eq!(
            validate_daily_alias_mapping(
                binding,
                &settings,
                DAILY_CONFIG_ALIAS,
                pin,
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            ),
            Err(Reason::CohortChanged)
        );
    }

    #[test]
    fn synthetic_calendar_suffix_ignores_old_prefix_gaps_but_requires_complete_suffix() {
        let candidates = [
            (date("2026-01-02"), DailySessionTypeV1::Trading),
            (date("2026-10-30"), DailySessionTypeV1::Trading),
            (date("2026-10-31"), DailySessionTypeV1::Closed),
            (date("2026-11-01"), DailySessionTypeV1::Closed),
            (date("2026-11-02"), DailySessionTypeV1::Trading),
            (date("2026-11-03"), DailySessionTypeV1::Closed),
            (date("2026-11-04"), DailySessionTypeV1::Trading),
        ];
        let selected = latest_trading_candidates(candidates.iter().copied(), date("2026-11-04"))
            .expect("three distinct verified trading candidates");
        assert_eq!(selected, expected_dates());

        let mut facts = std::collections::BTreeMap::new();
        // SYNTHETIC: a contradictory unrelated prefix is outside the selected
        // suffix; only verified production projections feed this helper.
        merge_calendar_suffix_claim(
            &mut facts,
            date("2026-01-02"),
            DailySessionTypeV1::Trading,
            selected[0],
            date("2026-11-04"),
        )
        .unwrap();
        merge_calendar_suffix_claim(
            &mut facts,
            date("2026-01-02"),
            DailySessionTypeV1::Closed,
            selected[0],
            date("2026-11-04"),
        )
        .unwrap();
        for (day, kind) in candidates
            .iter()
            .copied()
            .filter(|(day, _)| *day >= selected[0] && *day <= date("2026-11-04"))
        {
            merge_calendar_suffix_claim(&mut facts, day, kind, selected[0], date("2026-11-04"))
                .unwrap();
        }
        let suffix = complete_calendar_suffix(facts, selected[0], date("2026-11-04")).unwrap();
        assert_eq!(
            latest_three_trading_sessions(&suffix, date("2026-11-04")),
            Ok(selected)
        );

        let missing_inside_suffix = [
            (date("2026-10-30"), DailySessionTypeV1::Trading),
            (date("2026-10-31"), DailySessionTypeV1::Closed),
            (date("2026-11-02"), DailySessionTypeV1::Trading),
            (date("2026-11-03"), DailySessionTypeV1::Closed),
            (date("2026-11-04"), DailySessionTypeV1::Trading),
        ]
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            complete_calendar_suffix(
                missing_inside_suffix,
                date("2026-10-30"),
                date("2026-11-04"),
            ),
            Err(Reason::CalendarUnproven)
        );
        let mut conflicting_inside_suffix = std::collections::BTreeMap::new();
        merge_calendar_suffix_claim(
            &mut conflicting_inside_suffix,
            date("2026-11-01"),
            DailySessionTypeV1::Closed,
            date("2026-10-30"),
            date("2026-11-04"),
        )
        .unwrap();
        assert_eq!(
            merge_calendar_suffix_claim(
                &mut conflicting_inside_suffix,
                date("2026-11-01"),
                DailySessionTypeV1::Trading,
                date("2026-10-30"),
                date("2026-11-04"),
            ),
            Err(Reason::CalendarUnproven)
        );
    }

    #[test]
    fn synthetic_entitlement_selection_skips_foreign_batches_but_keeps_selected_errors() {
        let foreign_error = Err::<(), _>("foreign entitlement batch");
        let selected_error = Err::<(), _>("selected entitlement batch is malformed");
        let retained = selected_entitlement_batches([
            (false, foreign_error),
            (true, Ok(())),
            (true, selected_error),
        ])
        .collect::<Vec<_>>();
        assert_eq!(retained, [Ok(()), selected_error]);
    }

    #[test]
    fn synthetic_clock_rejects_negative_age_cutoff_reversal_and_monotonic_expiry() {
        let start = utc("2026-11-04T10:00:00Z");
        let mut negative_age = EvaluationClock::synthetic(start).unwrap();
        assert_eq!(
            negative_age.check_sample(
                start - chrono::Duration::milliseconds(200),
                std::time::Duration::from_millis(200),
            ),
            Err(Reason::EligibilityExpired)
        );

        let before_cutoff = utc("2026-11-04T07:29:59Z");
        let mut cutoff_reversal = EvaluationClock::synthetic(before_cutoff).unwrap();
        assert_eq!(
            cutoff_reversal.check_sample(
                utc("2026-11-04T07:30:01Z"),
                std::time::Duration::from_secs(2),
            ),
            Err(Reason::EligibilityExpired)
        );

        let mut monotonic_expiry = EvaluationClock::synthetic(start).unwrap();
        assert_eq!(
            monotonic_expiry.check_sample(
                start + chrono::Duration::milliseconds(179_500),
                std::time::Duration::from_millis(180_100),
            ),
            Err(Reason::EligibilityExpired)
        );

        let mut excessive_divergence = EvaluationClock::synthetic(start).unwrap();
        assert_eq!(
            excessive_divergence.check_sample(
                start + chrono::Duration::seconds(2),
                std::time::Duration::from_millis(500),
            ),
            Err(Reason::EligibilityExpired)
        );
    }

    #[test]
    fn synthetic_database_admission_and_action_links_match_resolved_evidence() {
        let linked = json!({"data":{"admission_sha256":"a".repeat(64)}});
        let admission = json!({"admission_sha256":"a".repeat(64)});
        assert!(admission_link_matches(&linked, &admission));
        let changed = json!({"admission_sha256":"b".repeat(64)});
        assert!(!admission_link_matches(&linked, &changed));
        assert!(!admission_link_matches(&json!({"data":{}}), &json!({})));

        let evidence = ActionScopeEvidence {
            range_start: "2026-10-01".to_owned(),
            range_end: "2026-10-30".to_owned(),
            page_batch_ids: vec![json!("batch-a"), json!("batch-a")],
        };
        assert!(action_source_matches(
            &json!({"batch_id":"batch-a","start":"2026-10-01","end":"2026-10-30"}),
            std::slice::from_ref(&evidence)
        ));
        assert!(!action_source_matches(
            &json!({"batch_id":"batch-b","start":"2026-10-01","end":"2026-10-30"}),
            std::slice::from_ref(&evidence)
        ));
        assert!(!action_source_matches(
            &json!({"batch_id":"batch-a","start":"2026-10-02","end":"2026-10-30"}),
            std::slice::from_ref(&evidence)
        ));
    }

    #[test]
    fn synthetic_holiday_gap_is_consecutive_and_valid_date_hashes_may_change() {
        let rows = [
            (date("2026-10-30"), DailySessionTypeV1::Trading),
            (date("2026-10-31"), DailySessionTypeV1::Closed),
            (date("2026-11-01"), DailySessionTypeV1::Closed),
            (date("2026-11-02"), DailySessionTypeV1::Trading),
            (date("2026-11-03"), DailySessionTypeV1::Closed),
            (date("2026-11-04"), DailySessionTypeV1::Trading),
        ];
        let selected = latest_three_trading_sessions(&rows, date("2026-11-04"))
            .expect("complete synthetic calendar with holidays");
        assert_eq!(selected, expected_dates());

        let cohort = cohort();
        let report = report(
            &sessions(&cohort),
            &selected,
            &cohort,
            chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 3, 0).unwrap(),
        );
        assert_eq!(report.decision(), PaperEligibilityDecision::Eligible);
        assert!(reason_codes(&report).is_empty());
        let encoded = report.encode_canonical().expect("bounded eligibility wire");
        let decoded: Value = serde_json::from_slice(&encoded).expect("eligibility JSON");
        assert_eq!(decoded["schema"], "owner-beta-paper-eligibility-v1");
        assert_eq!(decoded["decision"], "ELIGIBLE");
        assert_eq!(decoded["sessions"].as_array().unwrap().len(), MAX_SESSIONS);
        assert!(encoded.len() <= crate::owner_beta::paper::contract::PAPER_ELIGIBILITY_LIMIT);
    }

    #[test]
    fn synthetic_missing_calendar_day_and_failed_trading_session_break_the_streak() {
        let missing_day = [
            (date("2026-10-30"), DailySessionTypeV1::Trading),
            (date("2026-10-31"), DailySessionTypeV1::Closed),
            (date("2026-11-01"), DailySessionTypeV1::Closed),
            (date("2026-11-02"), DailySessionTypeV1::Trading),
            (date("2026-11-04"), DailySessionTypeV1::Trading),
        ];
        assert_eq!(
            latest_three_trading_sessions(&missing_day, date("2026-11-04")),
            Err(Reason::CalendarUnproven)
        );

        let cohort = cohort();
        let mut facts = sessions(&cohort);
        facts[1] = Err(Reason::SessionNotConsecutive);
        let result = report(
            &facts,
            &expected_dates(),
            &cohort,
            chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 1, 0).unwrap(),
        );
        assert_eq!(result.decision(), PaperEligibilityDecision::Ineligible);
        assert!(reason_codes(&result).contains(&"SESSION_NOT_CONSECUTIVE"));

        facts.pop();
        let result = report(
            &facts,
            &expected_dates(),
            &cohort,
            chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 1, 0).unwrap(),
        );
        assert!(reason_codes(&result).contains(&"CALENDAR_UNPROVEN"));
    }

    #[test]
    fn synthetic_manual_replay_duplicate_future_and_foreign_cohort_are_rejected() {
        let base = cohort();
        let expected = expected_dates();
        let expiry = chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 3, 0).unwrap();

        for mode in [
            SessionMode::Manual,
            SessionMode::CatchUp,
            SessionMode::Unknown,
        ] {
            let mut facts = sessions(&base);
            facts[1].as_mut().unwrap().mode = mode;
            assert!(
                reason_codes(&report(&facts, &expected, &base, expiry))
                    .contains(&"MANUAL_INTERVENTION")
            );
        }

        let mut facts = sessions(&base);
        facts[1].as_mut().unwrap().replayed = true;
        assert!(
            reason_codes(&report(&facts, &expected, &base, expiry)).contains(&"REPLAY_DETECTED")
        );

        let mut facts = sessions(&base);
        let duplicate_run = facts[0].as_ref().unwrap().run_identities.clone();
        facts[1].as_mut().unwrap().run_identities = duplicate_run;
        assert!(
            reason_codes(&report(&facts, &expected, &base, expiry)).contains(&"DUPLICATE_IDENTITY")
        );

        let mut facts = sessions(&base);
        let duplicate_source = facts[0].as_ref().unwrap().source_identities.clone();
        facts[1].as_mut().unwrap().source_identities = duplicate_source;
        assert!(
            reason_codes(&report(&facts, &expected, &base, expiry)).contains(&"DUPLICATE_IDENTITY")
        );

        let future = [date("2026-10-30"), date("2026-11-02"), date("2026-11-05")];
        let mut facts = sessions(&base);
        facts[2] = Ok(verified(future[2], &base, 2));
        assert!(reason_codes(&report(&facts, &future, &base, expiry)).contains(&"FUTURE_SESSION"));

        for (field, changed, expected_reason) in [
            (
                "owner_user_id",
                json!("11111111-1111-4111-8111-111111111111"),
                "OWNER_SCOPE_MISMATCH",
            ),
            (
                "strategy_config_id",
                json!("22222222-2222-4222-8222-222222222222"),
                "STRATEGY_BINDING_MISMATCH",
            ),
            (
                "strategy_config_sha256",
                json!(hash('c')),
                "STRATEGY_BINDING_MISMATCH",
            ),
            ("config_epoch", json!(9), "STRATEGY_BINDING_MISMATCH"),
            ("activation_epoch", json!(6), "COHORT_CHANGED"),
            ("cohort_sha256", json!(hash('d')), "COHORT_CHANGED"),
            ("policy_sha256", json!(hash('e')), "COHORT_CHANGED"),
            (
                "release_manifest_sha256",
                json!(hash('a')),
                "COHORT_CHANGED",
            ),
            ("entitlement_sha256", json!(hash('b')), "COHORT_CHANGED"),
        ] {
            let mut facts = sessions(&base);
            facts[1].as_mut().unwrap().cohort[field] = changed;
            assert!(
                reason_codes(&report(&facts, &expected, &base, expiry)).contains(&expected_reason)
            );
        }
    }

    #[test]
    fn synthetic_rights_terminal_revalidation_expiry_and_duplicate_originals_are_rejected() {
        let cohort = cohort();
        let expected = expected_dates();
        let expiry = chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 3, 0).unwrap();

        let mut facts = sessions(&cohort);
        facts[0].as_mut().unwrap().rights_active = false;
        facts[1].as_mut().unwrap().rights_sufficient = false;
        facts[2].as_mut().unwrap().source_revoked = true;
        let result = report(&facts, &expected, &cohort, expiry);
        assert!(reason_codes(&result).contains(&"RIGHTS_UNAVAILABLE"));
        assert!(reason_codes(&result).contains(&"SOURCE_REVOKED"));

        let mut facts = sessions(&cohort);
        facts[0].as_mut().unwrap().terminal_succeeded = false;
        facts[1].as_mut().unwrap().run_linked = false;
        facts[2].as_mut().unwrap().revalidated = false;
        let duplicate_original = facts[0].as_ref().unwrap().original_identities.clone();
        facts[2].as_mut().unwrap().original_identities = duplicate_original;
        let result = report(&facts, &expected, &cohort, expiry);
        assert!(reason_codes(&result).contains(&"SESSION_NOT_CONSECUTIVE"));
        assert!(reason_codes(&result).contains(&"EVIDENCE_TAMPERED"));
        assert!(reason_codes(&result).contains(&"DUPLICATE_IDENTITY"));

        let mut facts = sessions(&cohort);
        facts[0].as_mut().unwrap().input_admitted = false;
        facts[1].as_mut().unwrap().pins_match = false;
        facts[2].as_mut().unwrap().price_available = false;
        let result = report(&facts, &expected, &cohort, expiry);
        assert!(reason_codes(&result).contains(&"INPUT_NOT_ADMITTED"));
        assert!(reason_codes(&result).contains(&"INPUT_PIN_MISMATCH"));
        assert!(reason_codes(&result).contains(&"PRICE_NOT_AVAILABLE"));

        let expired = report(
            &sessions(&cohort),
            &expected,
            &cohort,
            chrono::Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap(),
        );
        assert_eq!(expired.decision(), PaperEligibilityDecision::Ineligible);
        assert!(reason_codes(&expired).contains(&"ELIGIBILITY_EXPIRED"));
    }
}

#[cfg(test)]
mod config_context_tests {
    use super::*;
    use market_data::{
        checked_close::{Attempt, Gap},
        raw_custody::RawInvocation,
    };
    use std::cell::Cell;

    #[test]
    fn first_alias_and_nonempty_policy_stops_typed_load() {
        assert!(alias_config_len_invalid(0));
        assert!(!alias_config_len_invalid(RAW_MANIFEST_BYTES as u64));
        assert!(alias_config_len_invalid(RAW_MANIFEST_BYTES as u64 + 1));
        let lower = Cell::new(0);
        let binding = serde_json::json!({});
        let settings = serde_json::json!({});
        let result = after_config_alias(
            || validate_daily_alias_mapping(&binding, &settings, DAILY_CONFIG_ALIAS, "pin", None),
            || {
                lower.set(lower.get() + 1);
                Ok::<_, Reason>(())
            },
        );
        assert_eq!(result.err(), Some(Reason::CohortChanged));
        assert_eq!(lower.get(), 0);
    }

    #[test]
    fn renewed_context_mismatch_keeps_initial_and_every_current_value() {
        struct Row(Box<str>);
        let initial = Some(Row("initial".into()));
        let initial_address = initial.as_ref().unwrap().0.as_ptr();
        let mut renewed = Vec::new();
        for name in ["renewed1", "renewed2", "renewed3"] {
            let current = Row(name.into());
            let address = current.0.as_ptr();
            let result = retain_context_revalidation(
                current,
                |current| {
                    if current.0 == initial.as_ref().unwrap().0 {
                        Ok(())
                    } else {
                        Err(Reason::CohortChanged)
                    }
                },
                &mut renewed,
            );
            assert_eq!(result.err(), Some(Reason::CohortChanged));
            assert_eq!(renewed.last().unwrap().0.as_ptr(), address);
        }
        assert_eq!(initial.as_ref().unwrap().0.as_ptr(), initial_address);
        assert_eq!(renewed.len(), 3);
    }

    #[test]
    fn actual_context_foreign_preflight_precedes_environment_and_file_reads() {
        let actual = Attempt::default();
        let foreign = Attempt::default();
        let invocation = RawInvocation::new(&actual);
        let mut session = ConfigSession::borrowed_in(&invocation, &foreign, &actual);
        let mut retained = ContextLoadRetention::default();
        let result = DailyContext::load_in(&mut session, &mut retained, &Value::Null);
        let failure = match result {
            Err(source) => source,
            Ok(_) => panic!("foreign scope"),
        };
        assert_eq!(failure.kind(), ConfigFirstKind::Scope);
        assert!(failure.reason().is_none() && failure.ingest().is_none());
        let completion = session.finish_local();
        assert_eq!(completion.read_count(), 0);
        assert_eq!(actual.report().attempted(), 0);
        assert_eq!(retained.partial_count(), 0);
        assert_eq!(retained.partials.len(), 1);
        assert!(failure.report(&completion).is_some());
        assert!(
            actual.report().secondary().iter().all(|issue| !matches!(
                issue,
                market_data::checked_close::Issue::Gap(Gap::NonLinux)
            ))
        );
        let debug = format!("{failure:?}");
        assert!(!debug.contains("Report") && !debug.contains("Value"));
    }
}

trait CalendarClaims {
    fn owner(&self) -> Uuid;
    fn policy_sha256(&self) -> &ContentHash;
    fn days(&self) -> &[market_data::daily_price_only::calendar::VerifiedCalendarDay];
}
impl CalendarClaims for VerifiedCalendarProjection<'_> {
    fn owner(&self) -> Uuid {
        self.owner()
    }
    fn policy_sha256(&self) -> &ContentHash {
        self.policy_sha256()
    }
    fn days(&self) -> &[market_data::daily_price_only::calendar::VerifiedCalendarDay] {
        self.days()
    }
}
#[cfg(target_os = "linux")]
impl CalendarClaims for market_data::daily_price_only::calendar::CalendarData {
    fn owner(&self) -> Uuid {
        self.owner()
    }
    fn policy_sha256(&self) -> &ContentHash {
        self.policy_sha256()
    }
    fn days(&self) -> &[market_data::daily_price_only::calendar::VerifiedCalendarDay] {
        self.days()
    }
}
trait CheckerSourcePolicy {
    type Error: From<Reason>;
    fn policy_first(&self) -> Result<&Value, Self::Error>;
    fn policy_event(&self, kind: &str, reason: Reason) -> Result<&Value, Self::Error>;
    fn policy_events(&self) -> Result<&[Value], Self::Error>;
    fn policy_checked_arm(&self) -> Result<Option<CheckedArmOrigin>, Self::Error> {
        Ok(None)
    }
}
impl CheckerSourcePolicy for ProtectedSourceChain {
    type Error = Reason;
    fn policy_first(&self) -> Result<&Value, Reason> {
        Ok(self.first())
    }
    fn policy_event(&self, kind: &str, reason: Reason) -> Result<&Value, Reason> {
        self.event(kind).map_err(|_| reason)
    }
    fn policy_events(&self) -> Result<&[Value], Reason> {
        Ok(self.events())
    }
}
trait CheckerRawPolicy {
    type Error: From<Reason>;
    fn matches_original(
        &mut self,
        reference: &SourceFileReference,
        evidence: &market_data::daily_price_only::DailyFileEvidenceV1Wire,
    ) -> Result<bool, Self::Error>;
}
impl CheckerRawPolicy for RawEvidenceSnapshots<'_> {
    type Error = Reason;
    fn matches_original(
        &mut self,
        reference: &SourceFileReference,
        evidence: &market_data::daily_price_only::DailyFileEvidenceV1Wire,
    ) -> Result<bool, Reason> {
        Ok(resolve_original(self, reference)
            .map_err(|_| Reason::EvidenceTampered)?
            .evidence()
            == evidence)
    }
}

/// The only live checker graph. It borrows the caller's pool; it never owns or closes it.
#[cfg(target_os = "linux")]
mod checker_custody {
    use super::super::input::{CheckedPaperInputFailure, validate_checked_input_in};
    use super::financial::ActionInputDriver;
    use super::*;
    use market_data::{
        CheckedHistoricalApproval, CheckedHistoricalApprovalCompletion,
        CheckedHistoricalApprovalFailure, HistoricalUseFailure,
        approve_historical_price_only_artifact_retained_checked,
        checked_close::{Attempt, Failure, Gap},
        daily_price_only::{
            DailyFileEvidenceV1Wire,
            admission::{
                CheckedDailyPolicyCompletion, CheckedDailyPolicyFailure, DailyPolicySource,
            },
            artifact::{
                CheckedDailyAdmissionRoot, CheckedDailyArtifactFailure, CheckedDailyCompletion,
            },
            calendar::{
                CheckedCalendarCompletion, CheckedCalendarFailure,
                project_calendar_candidates_checked,
            },
            replay::{CheckedReopenedDailyChain, read_sealed_daily_chain_checked},
            source::{
                ApprovedRawScope, CheckedRawSnapshot, CheckedRawSnapshotCompletion,
                CheckedRawSnapshotFailure, CheckedRawSnapshotUseFailure, CommittedRawReader,
                DailyEvidenceError, ScanLimits,
            },
            verify::{
                CheckedLineageData, CheckedLineageFailure, CheckedLineageLocal,
                CheckedRawEvidenceSnapshots, resolve_original_checked,
            },
        },
        paper_source_evidence::{
            CheckedCurrentCompletion, CheckedCurrentFailure, CheckedProtectedSourceChain,
            CheckedSourceCompletion, CheckedSourceFailure, ExecutionGateFailure,
            ProtectedCurrentBinding, ProtectedExecutionContext,
        },
        raw_custody::RawInvocation,
    };
    use std::{convert::Infallible, future::Future};
    type Unit = Result<(), Failure<Infallible>>;

    enum Source {
        Policy,
        Late,
        Sql(sqlx::Error),
        Current(Box<CheckedCurrentFailure>),
        Execution(Box<ExecutionGateFailure>),
        Config(Box<ContextConfigFailure>),
        Chain(Box<CheckedSourceFailure>),
        Raw(Box<CheckedRawSnapshotFailure>),
        RawUse(Box<CheckedRawSnapshotUseFailure>),
        RawPolicy(DailyEvidenceError),
        Lineage(Box<CheckedLineageFailure>),
        Calendar(Box<CheckedCalendarFailure>),
        Root(Box<CheckedDailyArtifactFailure>),
        Historical(Box<CheckedHistoricalApprovalFailure>),
        HistoricalUse(Box<HistoricalUseFailure>),
        Daily(Box<CheckedDailyPolicyFailure>),
        DailyUse(Box<DailyPolicySource>),
        Input(Box<CheckedPaperInputFailure>),
    }
    impl Source {
        fn kind(&self) -> &'static str {
            match self {
                Self::Policy => "policy",
                Self::Late => "late_only",
                Self::Sql(e) => {
                    let _ = e;
                    "sql"
                }
                Self::Current(e) => {
                    let _ = e;
                    "current"
                }
                Self::Execution(e) => {
                    let _ = e;
                    "execution"
                }
                Self::Config(e) => {
                    let _ = e;
                    "config"
                }
                Self::Chain(e) => {
                    let _ = e;
                    "source"
                }
                Self::Raw(e) => {
                    let _ = e;
                    "raw"
                }
                Self::RawUse(e) => {
                    let _ = e;
                    "raw_use"
                }
                Self::RawPolicy(e) => {
                    let _ = e;
                    "raw_policy"
                }
                Self::Lineage(e) => {
                    let _ = e;
                    "lineage"
                }
                Self::Calendar(e) => {
                    let _ = e;
                    "calendar"
                }
                Self::Root(e) => {
                    let _ = e;
                    "daily_root"
                }
                Self::Historical(e) => {
                    let _ = e;
                    "historical"
                }
                Self::HistoricalUse(e) => {
                    let _ = e;
                    "historical_use"
                }
                Self::Daily(e) => {
                    let _ = e;
                    "daily"
                }
                Self::DailyUse(e) => {
                    let _ = e;
                    "daily_use"
                }
                Self::Input(e) => {
                    let _ = e;
                    "input"
                }
            }
        }
    }
    pub(super) struct First {
        reason: Reason,
        source: Source,
    }
    impl From<Reason> for First {
        fn from(reason: Reason) -> Self {
            Self {
                reason,
                source: Source::Policy,
            }
        }
    }
    impl First {
        fn new(reason: Reason, source: Source) -> Self {
            Self { reason, source }
        }
        fn sql(error: sqlx::Error) -> Self {
            Self::new(sql_reason(&error), Source::Sql(error))
        }
        fn sql_setup(error: sqlx::Error) -> Self {
            Self::new(Reason::ReaderIncompatible, Source::Sql(error))
        }
    }
    enum Completion {
        Current(Box<CheckedCurrentCompletion>),
        Source(Box<CheckedSourceCompletion>),
        Raw(Box<CheckedRawSnapshotCompletion>),
        Root(Box<CheckedDailyCompletion>),
        Historical(
            Box<Result<CheckedHistoricalApprovalCompletion, CheckedHistoricalApprovalFailure>>,
        ),
        Daily(Box<CheckedDailyPolicyCompletion>),
        Calendar(Box<CheckedCalendarCompletion>),
        Config {
            retained: Box<ContextLoadRetention>,
            completion: Box<ConfigCompletion>,
        },
        Unit(Box<Unit>),
    }
    impl Completion {
        fn is_clean(&self) -> bool {
            match self {
                Self::Current(v) => v.is_clean(),
                Self::Source(v) => v.is_clean(),
                Self::Raw(v) => v.is_clean(),
                Self::Root(v) => v.is_clean(),
                Self::Historical(v) => v.is_ok(),
                Self::Daily(v) => v.is_clean(),
                Self::Calendar(v) => v.is_clean(),
                Self::Config {
                    retained,
                    completion,
                } => {
                    let _ = retained;
                    completion.is_clean()
                }
                Self::Unit(v) => v.is_ok(),
            }
        }
    }
    struct Observations {
        clean: bool,
        completions: Vec<Completion>,
        secondary: Vec<First>,
        rollback: Vec<Result<(), sqlx::Error>>,
        values: Vec<Value>,
        readbacks: Vec<UntrustedPaperSessionReadbackV1>,
        pins: Vec<DailyPricePinsV1Wire>,
        contexts: Vec<DailyContext>,
        lineage: Vec<CheckedLineageData>,
        locals: Vec<CheckedLineageLocal>,
        raw_references: Vec<SourceFileReference>,
        raw_aggregates: Vec<VerifiedDailyRawEvidence>,
        facts: Vec<VerifiedSessionFacts>,
        calendars: Vec<Vec<NaiveDate>>,
        current_binding: Option<Value>,
        financial_captures: Vec<financial::CapturedSession>,
        calendar_range_proof: Option<market_data::daily_price_only::calendar::CalendarRangeProof>,
        action_first: Option<financial::PaperInputActionError>,
        action_secondary: Vec<financial::PaperInputActionError>,
        action_committed: bool,
        action_commit_started: bool,
        action_retained: Vec<financial::RetainedActionValue>,
    }
    impl Observations {
        fn new() -> Self {
            Self {
                clean: true,
                completions: Vec::new(),
                secondary: Vec::new(),
                rollback: Vec::new(),
                values: Vec::new(),
                readbacks: Vec::new(),
                pins: Vec::new(),
                contexts: Vec::new(),
                lineage: Vec::new(),
                locals: Vec::new(),
                raw_references: Vec::new(),
                raw_aggregates: Vec::new(),
                facts: Vec::new(),
                calendars: Vec::new(),
                current_binding: None,
                financial_captures: Vec::new(),
                calendar_range_proof: None,
                action_first: None,
                action_secondary: Vec::new(),
                action_committed: false,
                action_commit_started: false,
                action_retained: Vec::new(),
            }
        }
        fn complete(&mut self, completion: Completion) {
            self.clean &= completion.is_clean();
            self.completions.push(completion);
        }
        fn local(&mut self, local: CheckedLineageLocal) {
            self.clean &= local.is_clean();
            self.locals.push(local);
        }
        fn require_clean(&self) -> Result<(), First> {
            if self.clean {
                Ok(())
            } else {
                Err(First::new(Reason::ReaderIncompatible, Source::Late))
            }
        }
    }
    fn retain_before_postcheck<T, E>(
        retained: &mut Vec<T>,
        returned: T,
        postcheck: impl FnOnce(&T) -> Result<(), E>,
    ) -> Result<(), E> {
        retained.push(returned);
        postcheck(retained.last().expect("actual returned value retained"))
    }
    pub(super) struct Outcome {
        original: Result<Option<PaperEligibilityReport>, First>,
        observations: Observations,
        final_clock: Result<DateTime<Utc>, Reason>,
    }
    fn publication<R, E>(
        original: Result<R, E>,
        clean: bool,
        final_clock: Result<(), Reason>,
        reason: impl FnOnce(E) -> Reason,
    ) -> Result<R, Reason> {
        match original {
            Err(first) => Err(reason(first)),
            Ok(value) => {
                final_clock?;
                if clean {
                    Ok(value)
                } else {
                    Err(Reason::ReaderIncompatible)
                }
            }
        }
    }
    impl std::fmt::Debug for Outcome {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CheckerOutcome")
                .field("original_ok", &self.original.is_ok())
                .field(
                    "source_kind",
                    &self.original.as_ref().err().map(|e| e.source.kind()),
                )
                .field(
                    "first_report_present",
                    &self
                        .original
                        .as_ref()
                        .err()
                        .and_then(|e| e.source_report(&self.observations))
                        .is_some(),
                )
                .field("completion_count", &self.observations.completions.len())
                .field("clean", &self.observations.clean)
                .finish_non_exhaustive()
        }
    }
    impl First {
        fn source_report<'a>(
            &'a self,
            observations: &'a Observations,
        ) -> Option<&'a market_data::checked_close::Report> {
            match &self.source {
                Source::Policy | Source::Late | Source::Sql(_) | Source::RawPolicy(_) => None,
                Source::Current(e) => e.diagnostics().source_report(),
                Source::Execution(e) => e.source_report(),
                Source::Config(e) => {
                    let completion = observations.completions.iter().find_map(|c| match c {
                        Completion::Config { completion, .. } => Some(completion),
                        _ => None,
                    })?;
                    e.report(completion)
                }
                Source::Chain(e) => e.diagnostics().source_report(),
                Source::Raw(e) => e.source_report(),
                Source::RawUse(e) => e.source_report(),
                Source::Lineage(e) => e.source_report(),
                Source::Calendar(e) => e.source_report(),
                Source::Root(e) => e.source_report(),
                Source::Historical(e) => e.first_report(),
                Source::HistoricalUse(e) => e.checks().source_report(),
                Source::Daily(e) => e.source_report(),
                Source::DailyUse(e) => e.report(),
                Source::Input(e) => e.source_report(),
            }
        }
    }
    impl Outcome {
        pub(super) fn publish(self) -> Result<PaperEligibilityReport, Reason> {
            let original = self
                .original
                .and_then(|report| report.ok_or_else(|| First::from(Reason::ReaderIncompatible)));
            publication(
                original,
                self.observations.clean,
                self.final_clock.map(|_| ()),
                |first| {
                    let _ = first.source.kind();
                    first.reason
                },
            )
        }

        pub(super) fn publish_capture(
            self,
            request: &financial::PaperCalculationInputRequest,
        ) -> Result<financial::ObservedPaperCalculationInputs, financial::PaperCalculationInputError>
        {
            let Self {
                original,
                observations,
                final_clock,
            } = self;
            let report = publication(
                original.and_then(|report| {
                    report.ok_or_else(|| First::from(Reason::ReaderIncompatible))
                }),
                observations.clean,
                final_clock.map(|_| ()),
                |first| {
                    let _ = first.source.kind();
                    first.reason
                },
            )
            .map_err(financial::PaperCalculationInputError::Eligibility)?;
            let requested_dates = request.price_dates().collect::<BTreeSet<_>>();
            let binding = observations
                .current_binding
                .ok_or(financial::PaperCalculationInputError::CaptureIncomplete)?;
            financial::publish_observed(
                report,
                binding,
                observations.financial_captures,
                observations.calendar_range_proof,
                &requested_dates,
                request.calendar_range().is_some(),
            )
        }

        pub(super) fn publish_action(mut self) -> ActionCapturePublication {
            // Codes are metadata derived by borrowing whole lower results.
            // The native outcome, including every completion and clock sample,
            // remains owned until the API maps/drops the local action result.
            let mut first = self.observations.action_first;
            let mut secondary = self.observations.action_secondary.clone();
            let committed = self.observations.action_committed;
            let commit_started = self.observations.action_commit_started;
            let mut retained = std::mem::take(&mut self.observations.action_retained);
            if let Err(failure) = &self.original {
                let error = financial::PaperInputActionError::Input(failure.reason);
                if first.is_none() {
                    first = Some(error);
                } else {
                    secondary.push(error);
                }
            }
            if !self.observations.clean {
                let error = if commit_started || committed {
                    financial::PaperInputActionError::CommitUnknown
                } else {
                    financial::PaperInputActionError::Cleanup
                };
                if first.is_none() {
                    first = Some(error);
                } else {
                    secondary.push(error);
                }
            }
            if self.final_clock.is_err() {
                let error = if commit_started || committed {
                    financial::PaperInputActionError::CommitUnknown
                } else {
                    financial::PaperInputActionError::Runtime
                };
                if first.is_none() {
                    first = Some(error);
                } else {
                    secondary.push(error);
                }
            }
            retained.push(financial::RetainedActionValue::LowerEvidence {
                _outcome: Box::new(self),
            });
            ActionCapturePublication {
                first,
                secondary,
                committed,
                commit_started,
                retained,
            }
        }
    }
    // Cancellation/panic claims only an actual gap. SQLx Drop is not awaited rollback proof.
    struct Unwind<'a> {
        attempt: &'a Attempt,
        armed: bool,
    }
    impl Drop for Unwind<'_> {
        fn drop(&mut self) {
            if self.armed {
                self.attempt.gap(Gap::Abandoned);
            }
        }
    }
    fn scopes(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        captured: &Attempt,
    ) -> (Unit, Unit) {
        let first = invocation.settle_local_in(expected, Ok::<(), Infallible>(()));
        let second = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
        (first, second)
    }
    struct Current<'c, 's, 'a> {
        context: &'c ProtectedExecutionContext<'s, 'a>,
        candidate: &'c UntrustedPaperContractV1,
        expected: &'c Value,
        config_selection: ConfigSelection,
    }
    impl Current<'_, '_, '_> {
        fn revalidate(&self) -> Result<(), First> {
            let actual = self
                .context
                .with_snapshot(|snapshot| {
                    authoritative_binding_from_snapshot(snapshot, self.candidate)
                })
                .map_err(|e| First::new(Reason::CohortChanged, Source::Execution(Box::new(e))))??;
            if actual != *self.expected {
                return Err(Reason::CohortChanged.into());
            }
            Ok(())
        }
        fn cohort(&self) -> &Value {
            &self.expected["cohort"]
        }
    }
    fn revalidate_retained_artifacts(
        root: &CheckedDailyAdmissionRoot<'_, '_>,
        historical: &CheckedHistoricalApproval<'_>,
        expected: &Attempt,
        captured: &Attempt,
    ) -> Result<(), First> {
        root.revalidate_in(expected, captured)
            .map_err(|error| First::new(Reason::EvidenceTampered, Source::Root(Box::new(error))))?;
        historical
            .approved_in(expected, captured)
            .map_err(|error| {
                First::new(
                    Reason::InputPinMismatch,
                    Source::HistoricalUse(Box::new(error)),
                )
            })?;
        historical
            .revalidate_in(expected, captured)
            .map_err(|error| {
                First::new(
                    Reason::EvidenceTampered,
                    Source::Historical(Box::new(error)),
                )
            })
    }
    pub(super) async fn run(
        pool: &PgPool,
        candidate: &UntrustedPaperContractV1,
        clock: &mut EvaluationClock,
        capture_request: Option<&financial::PaperCalculationInputRequest>,
        action_request: Option<&financial::PaperActionInputRequest>,
        outer_revalidate: Option<&mut dyn financial::ActionContextGuard>,
        action_consumer: Option<&mut dyn financial::LocalActionConsumer>,
    ) -> Outcome {
        run_selected(
            pool,
            None,
            Some(candidate),
            clock,
            capture_request,
            action_request,
            outer_revalidate,
            action_consumer,
        )
        .await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Native ENABLE derives its selector inside the protected Current snapshot."
    )]
    pub(super) async fn run_selected(
        pool: &PgPool,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        candidate: Option<&UntrustedPaperContractV1>,
        clock: &mut EvaluationClock,
        capture_request: Option<&financial::PaperCalculationInputRequest>,
        action_request: Option<&financial::PaperActionInputRequest>,
        outer_revalidate: Option<&mut dyn financial::ActionContextGuard>,
        action_consumer: Option<&mut dyn financial::LocalActionConsumer>,
    ) -> Outcome {
        let attempt = Attempt::default();
        let captured = attempt.clone();
        let mut unwind = Unwind {
            attempt: &attempt,
            armed: true,
        };
        let mut observations = Observations::new();
        let mut action_consumer = action_consumer;
        let mut outer_revalidate = outer_revalidate;
        let original = {
            let bootstrap = RawInvocation::new(&attempt);
            let result =
                match ProtectedCurrentBinding::open_checked_in(&bootstrap, &attempt, &captured) {
                    Err(e) => Err(First::new(
                        Reason::ReaderIncompatible,
                        Source::Current(Box::new(e)),
                    )),
                    Ok(current) => {
                        let current = current.into_execution_context();
                        let selected_candidate = match candidate {
                            Some(candidate) => Ok(candidate.clone()),
                            None => match current
                                .with_snapshot(|snapshot| snapshot.value().get("binding").cloned())
                            {
                                Err(error) => Err(First::new(
                                    Reason::CohortChanged,
                                    Source::Execution(Box::new(error)),
                                )),
                                Ok(None) => Err(Reason::CohortChanged.into()),
                                Ok(Some(value)) => wire::canonical(&value)
                                    .ok()
                                    .and_then(|bytes| UntrustedPaperContractV1::decode(&bytes).ok())
                                    .ok_or_else(|| First::from(Reason::CohortChanged)),
                            },
                        };
                        let original = match selected_candidate {
                            Err(first) => Err(first),
                            Ok(selected_candidate) => match current.with_snapshot(|snapshot| {
                                authoritative_binding_from_snapshot(snapshot, &selected_candidate)
                            }) {
                                Err(error) => Err(First::new(
                                    Reason::CohortChanged,
                                    Source::Execution(Box::new(error)),
                                )),
                                Ok(Err(reason)) => Err(reason.into()),
                                Ok(Ok(binding)) => {
                                    if capture_request.is_some() {
                                        observations.current_binding = Some(binding.clone());
                                    }
                                    let invocation = current.invocation();
                                    let guard = Current {
                                        context: &current,
                                        candidate: &selected_candidate,
                                        expected: &binding,
                                        config_selection: if candidate.is_none() {
                                            ConfigSelection::NativeCurrent
                                        } else {
                                            ConfigSelection::ServingEnvironment
                                        },
                                    };
                                    run_bound(
                                        pool,
                                        recovery_reader,
                                        &invocation,
                                        &attempt,
                                        &captured,
                                        &guard,
                                        clock,
                                        capture_request,
                                        action_request,
                                        &mut outer_revalidate,
                                        &mut action_consumer,
                                        &mut observations,
                                    )
                                    .await
                                }
                            },
                        };
                        // Original whole result is owned before this actual Current completion.
                        observations
                            .complete(Completion::Current(Box::new(current.finish_local())));
                        original
                    }
                };
            let (expected, captured) = scopes(&bootstrap, &attempt, &captured);
            observations.complete(Completion::Unit(Box::new(expected)));
            observations.complete(Completion::Unit(Box::new(captured)));
            result
        };
        unwind.armed = false;
        drop(unwind);
        observations.complete(Completion::Unit(Box::new(
            attempt.finish(Ok::<(), Infallible>(())),
        )));
        // This clock boundary is AFTER all real completion, so cleanup cannot publish expired eligibility.
        let final_clock = clock.checkpoint();
        Outcome {
            original,
            observations,
            final_clock,
        }
    }
    async fn finish_checker_transaction<T, E, F>(
        slot: Option<T>,
        rollback: impl FnOnce(T) -> F,
    ) -> Option<Result<(), E>>
    where
        F: Future<Output = Result<(), E>>,
    {
        match slot {
            Some(transaction) => Some(rollback(transaction).await),
            None => None,
        }
    }
    async fn rollback_owned(
        slot: &mut Option<Transaction<'static, Postgres>>,
        observations: &mut Observations,
    ) {
        if let Some(result) =
            finish_checker_transaction(slot.take(), |transaction| transaction.rollback()).await
        {
            observations.clean &= result.is_ok();
            observations.rollback.push(result);
        }
    }
    fn take_for_commit<T>(slot: &mut Option<T>) -> T {
        slot.take().expect("actual transaction slot")
    }
    fn finish_pair<T>(calendar: Option<T>, kis: Option<T>, mut finish: impl FnMut(T)) {
        if let Some(calendar) = calendar {
            finish(calendar);
        }
        if let Some(kis) = kis {
            finish(kis);
        }
    }
    struct Raw<'i> {
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        kis: Option<CheckedRawSnapshot<'i, 'i>>,
        calendar: Option<CheckedRawSnapshot<'i, 'i>>,
    }
    impl<'i> Raw<'i> {
        fn open(
            invocation: &'i RawInvocation<'i>,
            expected: &'i Attempt,
            captured: &'i Attempt,
            context: &DailyContext,
            observations: &mut Observations,
        ) -> Result<Self, First> {
            let mut raw = Self {
                invocation,
                expected,
                captured,
                kis: None,
                calendar: None,
            };
            let original = (|| {
                let reader = CommittedRawReader::new(&context.data_root);
                let kis_scope = ApprovedRawScope::new("kis", "kr", context.raw_owner_uid)
                    .map_err(|e| First::new(Reason::ReaderIncompatible, Source::RawPolicy(e)))?;
                let calendar_scope =
                    ApprovedRawScope::new("kis-calendar", "kr", context.raw_owner_uid).map_err(
                        |e| First::new(Reason::ReaderIncompatible, Source::RawPolicy(e)),
                    )?;
                raw.kis = Some(
                    reader
                        .snapshot_checked_in(
                            invocation,
                            expected,
                            captured,
                            kis_scope,
                            ScanLimits::new(context.scan_limits).map_err(|e| {
                                First::new(Reason::ReaderIncompatible, Source::RawPolicy(e))
                            })?,
                        )
                        .map_err(|e| {
                            First::new(Reason::EvidenceTampered, Source::Raw(Box::new(e)))
                        })?,
                );
                raw.calendar = Some(
                    reader
                        .snapshot_checked_in(
                            invocation,
                            expected,
                            captured,
                            calendar_scope,
                            ScanLimits::new(context.scan_limits).map_err(|e| {
                                First::new(Reason::ReaderIncompatible, Source::RawPolicy(e))
                            })?,
                        )
                        .map_err(|e| {
                            First::new(Reason::EvidenceTampered, Source::Raw(Box::new(e)))
                        })?,
                );
                Ok(())
            })();
            match original {
                Ok(()) => Ok(raw),
                Err(first) => {
                    raw.finish(observations);
                    Err(first)
                }
            }
        }
        fn snapshots<'s>(
            &'s self,
            context: &'s DailyContext,
            now: UtcTimestamp,
        ) -> Result<CheckedRawEvidenceSnapshots<'s, 'i, 'i>, First> {
            CheckedRawEvidenceSnapshots::new_in(
                self.invocation,
                self.expected,
                self.captured,
                &[
                    self.kis.as_ref().expect("KIS"),
                    self.calendar.as_ref().expect("calendar"),
                ],
                &context.entitlement_reference,
                now,
            )
            .map_err(|e| First::new(Reason::ReaderIncompatible, Source::Lineage(Box::new(e))))
        }
        fn finish(self, observations: &mut Observations) {
            finish_pair(self.calendar, self.kis, |snapshot| {
                observations.complete(Completion::Raw(Box::new(snapshot.finish_local())))
            });
        }
        fn compare(&self, refreshed: &Self, observations: &mut Observations) -> Result<(), First> {
            let checks = [
                self.kis
                    .as_ref()
                    .expect("KIS")
                    .view_in(self.expected, self.captured),
                self.calendar
                    .as_ref()
                    .expect("calendar")
                    .view_in(self.expected, self.captured),
                refreshed
                    .kis
                    .as_ref()
                    .expect("KIS")
                    .view_in(self.expected, self.captured),
                refreshed
                    .calendar
                    .as_ref()
                    .expect("calendar")
                    .view_in(self.expected, self.captured),
            ];
            let mut views = Vec::new();
            let mut first = None;
            for result in checks {
                match result {
                    Ok(view) => views.push(view),
                    Err(e) => {
                        let error =
                            First::new(Reason::EvidenceTampered, Source::RawUse(Box::new(e)));
                        if first.is_none() {
                            first = Some(error);
                        } else {
                            observations.secondary.push(error);
                        }
                    }
                }
            }
            if let Some(first) = first {
                return Err(first);
            }
            if views[2].manifest_sha256() != views[0].manifest_sha256()
                || views[3].manifest_sha256() != views[1].manifest_sha256()
            {
                return Err(Reason::EvidenceTampered.into());
            }
            Ok(())
        }
        #[expect(
            clippy::too_many_arguments,
            reason = "Retained Raw evidence, exact dates and action calendar policy remain explicit."
        )]
        fn refresh(
            &self,
            context: &DailyContext,
            clock: &mut EvaluationClock,
            dates: &[NaiveDate],
            references: &[SourceFileReference],
            calendar_range: Option<(TradingDate, TradingDate)>,
            action_request: Option<&financial::PaperActionInputRequest>,
            observations: &mut Observations,
        ) -> Result<(), First> {
            clock.checkpoint()?;
            let refreshed = Self::open(
                self.invocation,
                self.expected,
                self.captured,
                context,
                observations,
            )?;
            let original: Result<(), First> = (|| {
                self.compare(&refreshed, observations)?;
                let calendar_at = clock.checkpoint()?;
                if discover(
                    &refreshed,
                    context,
                    calendar_at,
                    calendar_range,
                    action_request,
                    observations,
                )? != *dates
                {
                    return Err(Reason::CalendarUnproven.into());
                }
                for reference in references {
                    let at = clock.checkpoint()?;
                    let snapshots =
                        refreshed.snapshots(context, UtcTimestamp::from_datetime(at))?;
                    let result = resolve_original_checked(&snapshots, reference).map_err(|e| {
                        First::new(Reason::EvidenceTampered, Source::Lineage(Box::new(e)))
                    });
                    let result = match result {
                        Ok(original) => {
                            observations.lineage.push(original.retain_diagnostics());
                            Ok(())
                        }
                        Err(e) => Err(e),
                    };
                    observations.local(snapshots.into_diagnostics());
                    result?;
                }
                clock.checkpoint()?;
                Ok(())
            })();
            refreshed.finish(observations);
            original?;
            observations.require_clean()
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Custody lifetimes and optional financial capture share the existing checked traversal"
    )]
    async fn run_bound<'i>(
        pool: &PgPool,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        current: &Current<'_, '_, '_>,
        clock: &mut EvaluationClock,
        capture_request: Option<&financial::PaperCalculationInputRequest>,
        action_request: Option<&financial::PaperActionInputRequest>,
        outer_revalidate: &mut Option<&mut dyn financial::ActionContextGuard>,
        action_consumer: &mut Option<&mut dyn financial::LocalActionConsumer>,
        observations: &mut Observations,
    ) -> Result<Option<PaperEligibilityReport>, First> {
        let mut config = ConfigSession::borrowed_in(invocation, expected, captured);
        let mut retained = ContextLoadRetention {
            selection: current.config_selection,
            ..ContextLoadRetention::default()
        };
        let initial = (|| {
            clock.checkpoint()?;
            let evaluated_at = clock.started_at;
            let eligible_date = clock.eligible_date.as_naive_date();
            let expires_at = eligibility_expiry(evaluated_at, eligible_date)
                .ok_or(Reason::EligibilityExpired)?;
            current.revalidate()?;
            let context = DailyContext::load_in(&mut config, &mut retained, current.expected)
                .map_err(|e| {
                    First::new(
                        e.reason().copied().unwrap_or(Reason::ReaderIncompatible),
                        Source::Config(Box::new(e)),
                    )
                })?;
            Ok::<_, First>((context, evaluated_at, eligible_date, expires_at))
        })();
        let original = match initial {
            Err(first) => Err(first),
            Ok((context, evaluated_at, eligible_date, expires_at)) => {
                let result = run_context(
                    pool,
                    recovery_reader,
                    invocation,
                    expected,
                    captured,
                    current,
                    clock,
                    capture_request,
                    action_request,
                    outer_revalidate,
                    action_consumer,
                    &context,
                    &mut config,
                    &mut retained,
                    evaluated_at,
                    eligible_date,
                    expires_at,
                    observations,
                )
                .await;
                observations.contexts.push(context);
                result
            }
        };
        observations.complete(Completion::Config {
            retained: Box::new(retained),
            completion: Box::new(config.finish_local()),
        });
        original
    }
    fn admitted_price_input<'a>(
        session: &'a Session<'_, '_>,
    ) -> Result<financial::PaperAdmittedPriceObservation<'a>, First> {
        let snapshot = session
            .financial_snapshot
            .as_ref()
            .ok_or(Reason::PriceNotAvailable)?;
        let readback = session
            .financial_readback
            .as_ref()
            .ok_or(Reason::EvidenceMissing)?;
        let availability = session
            .financial_availability
            .ok_or(Reason::PriceNotAvailable)?;
        let observed_at = session
            .financial_observed_at
            .ok_or(Reason::PriceNotAvailable)?;
        Ok(financial::PaperAdmittedPriceObservation::new(
            snapshot,
            readback,
            availability,
            observed_at,
        ))
    }

    fn checked_action_inputs<'a>(
        current_binding: &'a Value,
        action_request: &financial::PaperActionInputRequest,
        calculation_request: &financial::PaperCalculationInputRequest,
        dates: &[NaiveDate],
        sessions: &'a [Session<'_, '_>],
        range_proof: Option<&'a market_data::daily_price_only::calendar::CalendarRangeProof>,
        evaluated_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<financial::PaperActionInputs<'a>, First> {
        let signal_date = action_request
            .signal_date()
            .map(|date| date.as_naive_date())
            .or_else(|| dates.first().copied())
            .ok_or(Reason::CalendarUnproven)?;
        let signal_session = sessions
            .iter()
            .find(|session| session.date == signal_date)
            .ok_or(Reason::EvidenceMissing)?;
        let signal_snapshot = signal_session
            .financial_snapshot
            .as_ref()
            .ok_or(Reason::PriceNotAvailable)?;
        let signal_readback = signal_session
            .financial_readback
            .as_ref()
            .ok_or(Reason::EvidenceMissing)?;
        let signal_availability = signal_session
            .financial_availability
            .ok_or(Reason::PriceNotAvailable)?;
        let signal_observed_at = signal_session
            .financial_observed_at
            .ok_or(Reason::PriceNotAvailable)?;
        let signal_input = signal_session
            .financial_input
            .as_ref()
            .ok_or(Reason::EvidenceMissing)?;
        let signal_target = signal_session
            .financial_target
            .as_ref()
            .ok_or(Reason::EvidenceMissing)?;
        let signal_prices = admitted_price_input(signal_session)?;
        let signal = financial::PaperSignalObservation::new(
            signal_snapshot,
            signal_readback,
            signal_prices,
            signal_availability,
            signal_observed_at,
            signal_input,
            signal_target,
        );
        let execution = action_request
            .execution_date()
            .map(|date| {
                sessions
                    .iter()
                    .find(|session| session.date == date.as_naive_date())
                    .ok_or_else(|| First::from(Reason::EvidenceMissing))
                    .and_then(admitted_price_input)
            })
            .transpose()?;
        let native_enable_sessions = if !calculation_request.is_recovery_selection() {
            if sessions.len() != MAX_SESSIONS {
                return Err(Reason::EvidenceMissing.into());
            }
            let mut native_enable_sessions = sessions
                .iter()
                .map(|session| {
                    session
                        .native_enable_session
                        .as_ref()
                        .map(|fact| (session.date, fact))
                        .ok_or_else(|| First::from(Reason::EvidenceMissing))
                })
                .collect::<Result<Vec<_>, _>>()?;
            native_enable_sessions.sort_by_key(|(date, _)| *date);
            if native_enable_sessions
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
            {
                return Err(Reason::DuplicateIdentity.into());
            }
            native_enable_sessions
                .into_iter()
                .map(|(_, fact)| fact)
                .collect()
        } else {
            // The private recovery selector has checked original T plus exact E.
            // It never manufactures ENABLE's three-session proof; all ordinary
            // selectors retain their existing gate above.
            Vec::new()
        };
        Ok(financial::PaperActionInputs::new(
            current_binding,
            signal,
            execution,
            range_proof,
            native_enable_sessions,
            evaluated_at,
            expires_at,
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Genuine borrowed holders, actual scope endpoints and retained results remain explicit."
    )]
    async fn run_context<'i>(
        pool: &PgPool,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        current: &Current<'_, '_, '_>,
        clock: &mut EvaluationClock,
        capture_request: Option<&financial::PaperCalculationInputRequest>,
        action_request: Option<&financial::PaperActionInputRequest>,
        outer_revalidate: &mut Option<&mut dyn financial::ActionContextGuard>,
        action_consumer: &mut Option<&mut dyn financial::LocalActionConsumer>,
        context: &DailyContext,
        config: &mut ConfigSession<'i, 'i>,
        retained: &mut ContextLoadRetention,
        evaluated_at: DateTime<Utc>,
        eligible_date: NaiveDate,
        expires_at: DateTime<Utc>,
        observations: &mut Observations,
    ) -> Result<Option<PaperEligibilityReport>, First> {
        current.revalidate()?;
        clock.checkpoint()?;
        current.revalidate()?;
        let raw = Raw::open(invocation, expected, captured, context, observations)?;
        let mut root = None;
        let mut historical = None;
        let original = async {
            current.revalidate()?;
            let observed_at = clock.checkpoint()?;
            let timestamp = UtcTimestamp::from_datetime(observed_at);
            current.revalidate()?;
            let snapshots = raw.snapshots(context, timestamp)?;
            let result = async {
                current.revalidate()?;
                let dates = discover(
                    &raw,
                    context,
                    observed_at,
                    capture_request.and_then(|request| request.calendar_range()),
                    action_request,
                    observations,
                )?;
                current.revalidate()?;
                clock.checkpoint()?;
                current.revalidate()?;
                root = Some(
                    CheckedDailyAdmissionRoot::open_in(
                        invocation,
                        expected,
                        captured,
                        context.config.admission_path(),
                        DAILY_ADMITTER_UID,
                        DAILY_READER_GROUP,
                    )
                    .map_err(|e| {
                        First::new(Reason::ReaderIncompatible, Source::Root(Box::new(e)))
                    })?,
                );
                historical = Some(
                    approve_historical_price_only_artifact_retained_checked(
                        invocation,
                        expected,
                        captured,
                        context.config.historical_path(),
                    )
                    .map_err(|e| {
                        First::new(Reason::ReaderIncompatible, Source::Historical(Box::new(e)))
                    })?,
                );
                current.revalidate()?;
                run_sessions(
                    pool,
                    recovery_reader,
                    invocation,
                    expected,
                    captured,
                    current,
                    clock,
                    context,
                    config,
                    retained,
                    &raw,
                    &snapshots,
                    root.as_ref().expect("actual Daily root"),
                    historical.as_ref().expect("actual historical approval"),
                    dates,
                    capture_request,
                    action_request,
                    outer_revalidate,
                    action_consumer,
                    evaluated_at,
                    eligible_date,
                    expires_at,
                    observations,
                )
                .await
            }
            .await;
            observations.local(snapshots.into_diagnostics());
            result
        }
        .await;
        // All session borrows ended. Do not short circuit any actual completion.
        if let Some(root) = root {
            observations.complete(Completion::Root(Box::new(root.finish_local())));
        }
        if let Some(historical) = historical {
            observations.complete(Completion::Historical(Box::new(historical.finish_local())));
        }
        raw.finish(observations);
        original
    }
    struct Session<'env, 'i> {
        date: NaiveDate,
        cohort: Value,
        source: Option<CheckedProtectedSourceChain<'i, 'i>>,
        chain: Option<CheckedReopenedDailyChain<'env, 'i, 'i, 'i>>,
        pins: Option<DailyPricePinsV1Wire>,
        facts: Option<VerifiedSessionFacts>,
        raw_references: Vec<SourceFileReference>,
        raw_aggregates: Vec<VerifiedDailyRawEvidence>,
        database_readback_sha256: Option<String>,
        database_admission_projection_sha256: Option<String>,
        native_enable_session: Option<CheckedEnableSession>,
        capture_financial: bool,
        recommendation_required: bool,
        expected_run_id: Option<Uuid>,
        financial_snapshot: Option<financial::PaperPriceSnapshotV1>,
        financial_readback: Option<UntrustedPaperSessionReadbackV1>,
        financial_observed_at: Option<DateTime<Utc>>,
        financial_availability: Option<financial::PriceAvailabilityObservation>,
        financial_input:
            Option<crate::owner_beta::daily_input::OwnerBetaDailyRecommendationInputV1>,
        financial_target: Option<crate::owner_beta::daily_target::OwnerBetaDailyTargetSnapshotV1>,
    }
    impl<'env, 'i> Session<'env, 'i> {
        fn new(
            date: NaiveDate,
            cohort: &Value,
            capture_financial: bool,
            recommendation_required: bool,
            expected_run_id: Option<Uuid>,
        ) -> Self {
            Self {
                date,
                cohort: cohort.clone(),
                source: None,
                chain: None,
                pins: None,
                facts: None,
                raw_references: Vec::new(),
                raw_aggregates: Vec::new(),
                database_readback_sha256: None,
                database_admission_projection_sha256: None,
                native_enable_session: None,
                capture_financial,
                recommendation_required,
                expected_run_id,
                financial_snapshot: None,
                financial_readback: None,
                financial_observed_at: None,
                financial_availability: None,
                financial_input: None,
                financial_target: None,
            }
        }
        fn revalidate(
            &self,
            invocation: &RawInvocation<'_>,
            expected: &Attempt,
            captured: &Attempt,
        ) -> Result<(), First> {
            let source = self.source.as_ref().expect("retained Source");
            source
                .check_scope_in(invocation, expected, captured)
                .map_err(|e| First::new(Reason::EvidenceTampered, Source::Chain(Box::new(e))))?;
            source
                .revalidate()
                .map_err(|e| First::new(Reason::EvidenceTampered, Source::Chain(Box::new(e))))?;
            super::super::input::revalidate_checked_input_in(
                self.chain.as_ref().expect("retained chain"),
                expected,
                captured,
            )
            .map_err(|e| First::new(Reason::InputPinMismatch, Source::Input(Box::new(e))))
        }
        fn finish(self, observations: &mut Observations) {
            // Whole returned ordinary data survives all real chain/Source retirement.
            // Separate borrows end between the two callbacks; neither completion is skipped.
            let mut chain_completion = None;
            let mut source_completion = None;
            finish_session_resources(
                self.chain,
                self.source,
                |chain| chain_completion = Some(chain.finish_local()),
                |source| source_completion = Some(source.finish_local()),
            );
            if let Some(completion) = chain_completion {
                observations.complete(Completion::Daily(Box::new(completion)));
            }
            if let Some(completion) = source_completion {
                observations.complete(Completion::Source(Box::new(completion)));
            }
            if let Some(pins) = self.pins {
                observations.pins.push(pins);
            }
            if let Some(facts) = self.facts {
                observations.facts.push(facts);
            }
            if self.capture_financial
                && let (Some(snapshot), Some(readback), Some(observed_at), Some(availability)) = (
                    self.financial_snapshot,
                    self.financial_readback,
                    self.financial_observed_at,
                    self.financial_availability,
                )
            {
                observations
                    .financial_captures
                    .push(financial::CapturedSession {
                        date: self.date,
                        snapshot,
                        readback,
                        observed_at,
                        availability,
                        input: self.financial_input,
                        target: self.financial_target,
                    });
            }
            observations.raw_references.extend(self.raw_references);
            observations.raw_aggregates.extend(self.raw_aggregates);
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ActionTransactionPhase {
        NotStarted,
        Active,
        RolledBack,
        CommitStarted,
        Committed,
        CommitUnknown,
    }

    #[derive(Default)]
    struct ActionFailures {
        dirty: bool,
        first: Option<financial::PaperInputActionError>,
        secondary: Vec<financial::PaperInputActionError>,
    }

    impl ActionFailures {
        fn fail(
            &mut self,
            error: financial::PaperInputActionError,
        ) -> financial::PaperInputActionError {
            self.dirty = true;
            if let Some(first) = self.first {
                if first != error {
                    self.secondary.push(error);
                }
                first
            } else {
                self.first = Some(error);
                error
            }
        }

        fn fail_first(
            &mut self,
            first: First,
            observations: &mut Observations,
        ) -> financial::PaperInputActionError {
            let error = financial::PaperInputActionError::Input(first.reason);
            observations.secondary.push(first);
            self.fail(error)
        }
    }

    fn checkpoint_action_date(
        clock: &mut EvaluationClock,
        action_started_seoul_date: NaiveDate,
    ) -> Result<(), financial::PaperInputActionError> {
        let checked_at = clock
            .checkpoint()
            .map_err(financial::PaperInputActionError::Input)?;
        let kst = FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul civil offset");
        if checked_at.with_timezone(&kst).date_naive() != action_started_seoul_date {
            return Err(financial::PaperInputActionError::Input(
                Reason::EligibilityExpired,
            ));
        }
        Ok(())
    }

    /// The actual last synchronous boundary after every awaited final seal.
    /// Keep transaction ownership and Active phase on refusal so finish_scope
    /// can roll back; no IO or another async validation window opens here.
    fn take_after_commit_boundary<T>(
        outer: &mut dyn financial::ActionContextGuard,
        check_date: impl FnOnce() -> Result<(), financial::PaperInputActionError>,
        phase: &mut ActionTransactionPhase,
        transaction: &mut Option<T>,
        failures: &mut ActionFailures,
    ) -> Result<T, financial::PaperInputActionError> {
        outer
            .checkpoint()
            .and_then(|_| check_date())
            .map_err(|error| failures.fail(error))?;
        if failures.dirty {
            return Err(failures
                .first
                .unwrap_or(financial::PaperInputActionError::TransactionState));
        }
        if *phase != ActionTransactionPhase::Active || transaction.is_none() {
            return Err(failures.fail(financial::PaperInputActionError::TransactionState));
        }
        *phase = ActionTransactionPhase::CommitStarted;
        Ok(take_for_commit(transaction))
    }

    struct ScopedActionDriver<'a, 'env, 'i, 'c, 's, 'p> {
        pool: &'a PgPool,
        recovery_reader: Option<&'a NativeRecoveryReadContext>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        current: &'a Current<'c, 's, 'p>,
        clock: &'a mut EvaluationClock,
        context: &'a DailyContext,
        config: &'a mut ConfigSession<'i, 'i>,
        retained: &'a mut ContextLoadRetention,
        raw: &'a Raw<'i>,
        root: &'a CheckedDailyAdmissionRoot<'i, 'i>,
        historical: &'a CheckedHistoricalApproval<'i>,
        sessions: &'a [Session<'env, 'i>],
        dates: &'a [NaiveDate],
        references: &'a [SourceFileReference],
        calculation_request: &'a financial::PaperCalculationInputRequest,
        action_request: &'a financial::PaperActionInputRequest,
        range_proof: Option<&'a market_data::daily_price_only::calendar::CalendarRangeProof>,
        action_started_seoul_date: NaiveDate,
        outer_revalidate: &'a mut dyn financial::ActionContextGuard,
        observations: &'a mut Observations,
        transaction: Option<Transaction<'static, Postgres>>,
        phase: ActionTransactionPhase,
        commit_succeeded: bool,
        failures: ActionFailures,
        retained_values: Vec<financial::RetainedActionValue>,
    }

    impl ScopedActionDriver<'_, '_, '_, '_, '_, '_> {
        fn check_action_date(&mut self) -> Result<(), financial::PaperInputActionError> {
            checkpoint_action_date(self.clock, self.action_started_seoul_date)
                .map_err(|error| self.fail(error))
        }

        fn fail(
            &mut self,
            error: financial::PaperInputActionError,
        ) -> financial::PaperInputActionError {
            self.failures.fail(error)
        }

        fn fail_first(&mut self, first: First) -> financial::PaperInputActionError {
            self.failures.fail_first(first, self.observations)
        }

        fn check_files(&mut self) -> Result<(), financial::PaperInputActionError> {
            let result = self.check_files_inner();
            match result {
                Ok(()) => Ok(()),
                Err(error) => Err(self.fail(error)),
            }
        }

        fn check_files_inner(&mut self) -> Result<(), financial::PaperInputActionError> {
            self.outer_revalidate
                .revalidate()
                .map_err(|error| self.fail(error))?;
            self.check_action_date()?;
            self.current
                .revalidate()
                .map_err(|error| self.fail_first(error))?;
            revalidate_retained_artifacts(self.root, self.historical, self.expected, self.captured)
                .map_err(|error| self.fail_first(error))?;
            for session in self.sessions {
                self.clock
                    .checkpoint()
                    .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
                session
                    .revalidate(self.invocation, self.expected, self.captured)
                    .map_err(|error| self.fail_first(error))?;
            }
            self.context
                .revalidate_in(&mut *self.config, self.retained, self.current.expected)
                .map_err(|error| {
                    self.fail_first(First::new(
                        error
                            .reason()
                            .copied()
                            .unwrap_or(Reason::ReaderIncompatible),
                        Source::Config(Box::new(error)),
                    ))
                })?;
            self.clock
                .checkpoint()
                .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
            self.check_action_date()?;
            let raw = self.raw;
            let context = self.context;
            let clock = &mut self.clock;
            let dates = self.dates;
            let references = self.references;
            let range = self.calculation_request.calendar_range();
            let action = self.action_request;
            let observations = &mut self.observations;
            raw.refresh(
                context,
                clock,
                dates,
                references,
                range,
                Some(action),
                observations,
            )
            .map_err(|error| self.fail_first(error))?;
            let refreshed = self.observations.calendar_range_proof.take();
            let same = match (self.range_proof, refreshed.as_ref()) {
                (None, None) => true,
                (Some(expected), Some(actual)) => same_calendar_range_proof(expected, actual),
                _ => false,
            };
            if !same {
                return Err(self.fail(financial::PaperInputActionError::Input(
                    Reason::CalendarUnproven,
                )));
            }
            self.current
                .revalidate()
                .map_err(|error| self.fail_first(error))?;
            self.clock
                .checkpoint()
                .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
            self.check_action_date()?;
            if self.failures.dirty {
                return Err(self
                    .failures
                    .first
                    .unwrap_or(financial::PaperInputActionError::Cleanup));
            }
            Ok(())
        }

        async fn check_files_with_fence(
            &mut self,
            fence: &mut Option<&mut dyn financial::PaperActionCommitFence>,
        ) -> Result<(), financial::PaperInputActionError> {
            if fence.is_none() {
                return self.check_files();
            }
            if self.failures.dirty {
                return Err(self
                    .failures
                    .first
                    .unwrap_or(financial::PaperInputActionError::Cleanup));
            }
            self.checkpoint_fence(fence).await?;
            self.outer_revalidate
                .revalidate()
                .map_err(|error| self.fail(error))?;
            self.checkpoint_fence(fence).await?;
            self.check_action_date()?;
            self.current
                .revalidate()
                .map_err(|error| self.fail_first(error))?;
            self.checkpoint_fence(fence).await?;
            revalidate_retained_artifacts(self.root, self.historical, self.expected, self.captured)
                .map_err(|error| self.fail_first(error))?;
            self.checkpoint_fence(fence).await?;
            for session in self.sessions {
                self.clock
                    .checkpoint()
                    .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
                session
                    .revalidate(self.invocation, self.expected, self.captured)
                    .map_err(|error| self.fail_first(error))?;
                self.checkpoint_fence(fence).await?;
            }
            self.context
                .revalidate_in(&mut *self.config, self.retained, self.current.expected)
                .map_err(|error| {
                    self.fail_first(First::new(
                        error
                            .reason()
                            .copied()
                            .unwrap_or(Reason::ReaderIncompatible),
                        Source::Config(Box::new(error)),
                    ))
                })?;
            self.checkpoint_fence(fence).await?;
            self.clock
                .checkpoint()
                .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
            self.check_action_date()?;
            let raw = self.raw;
            let context = self.context;
            let clock = &mut self.clock;
            let dates = self.dates;
            let references = self.references;
            let range = self.calculation_request.calendar_range();
            let action = self.action_request;
            let observations = &mut self.observations;
            raw.refresh(
                context,
                clock,
                dates,
                references,
                range,
                Some(action),
                observations,
            )
            .map_err(|error| self.fail_first(error))?;
            self.checkpoint_fence(fence).await?;
            let refreshed = self.observations.calendar_range_proof.take();
            let same = match (self.range_proof, refreshed.as_ref()) {
                (None, None) => true,
                (Some(expected), Some(actual)) => same_calendar_range_proof(expected, actual),
                _ => false,
            };
            if !same {
                return Err(self.fail(financial::PaperInputActionError::Input(
                    Reason::CalendarUnproven,
                )));
            }
            self.current
                .revalidate()
                .map_err(|error| self.fail_first(error))?;
            self.checkpoint_fence(fence).await?;
            self.clock
                .checkpoint()
                .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
            self.check_action_date()?;
            if self.failures.dirty {
                return Err(self
                    .failures
                    .first
                    .unwrap_or(financial::PaperInputActionError::Cleanup));
            }
            self.checkpoint_fence(fence).await?;
            Ok(())
        }

        async fn readback_on_action_transaction(
            &mut self,
        ) -> Result<(), financial::PaperInputActionError> {
            self.readback_with_fence(&mut None).await
        }

        async fn checkpoint_fence(
            &mut self,
            fence: &mut Option<&mut dyn financial::PaperActionCommitFence>,
        ) -> Result<(), financial::PaperInputActionError> {
            let Some(fence) = fence.as_deref_mut() else {
                return Ok(());
            };
            let transaction = self
                .transaction
                .as_mut()
                .ok_or(financial::PaperInputActionError::TransactionState)?;
            if let Err(error) = fence.checkpoint(transaction).await {
                self.retained_values
                    .push(financial::RetainedActionValue::QueueError(error));
                return Err(self.fail(financial::PaperInputActionError::Runtime));
            }
            Ok(())
        }

        async fn readback_with_fence(
            &mut self,
            fence: &mut Option<&mut dyn financial::PaperActionCommitFence>,
        ) -> Result<(), financial::PaperInputActionError> {
            if self.phase != ActionTransactionPhase::Active {
                return Err(self.fail(financial::PaperInputActionError::TransactionState));
            }
            self.checkpoint_fence(fence).await?;
            self.outer_revalidate
                .revalidate()
                .map_err(|error| self.fail(error))?;
            self.checkpoint_fence(fence).await?;
            self.clock
                .checkpoint()
                .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
            self.current
                .revalidate()
                .map_err(|error| self.fail_first(error))?;
            self.checkpoint_fence(fence).await?;
            let mut order = self.sessions.iter().collect::<Vec<_>>();
            order.sort_by_key(|session| session.date);
            for session in order {
                let cohort = &session.cohort;
                let date = session.date;
                let worker_recovery = self.calculation_request.worker_recovery.as_ref();
                let receipt =
                    worker_recovery.and_then(|worker| worker.receipt(&self.retained_values));
                let result = read_session_database(
                    self.transaction
                        .as_mut()
                        .expect("active economic transaction"),
                    self.recovery_reader,
                    worker_recovery,
                    date,
                    cohort,
                    receipt.as_deref(),
                )
                .await;
                let value = match result {
                    Ok(value) => value,
                    Err(error) => {
                        self.retained_values
                            .push(financial::RetainedActionValue::SqlError(error));
                        return Err(self.fail(financial::PaperInputActionError::Sql));
                    }
                };
                self.retained_values
                    .push(financial::RetainedActionValue::SqlValue(value.clone()));
                self.checkpoint_fence(fence).await?;
                self.observations.values.push(value.clone());
                let readback = match UntrustedPaperSessionReadbackV1::from_value(value) {
                    Ok(readback) => readback,
                    Err(_) => {
                        return Err(self.fail(financial::PaperInputActionError::Input(
                            Reason::EvidenceTampered,
                        )));
                    }
                };
                self.observations.readbacks.push(readback);
                let slot = wire::slot_id(cohort, &date.to_string()).map_err(|_| {
                    self.fail(financial::PaperInputActionError::Input(
                        Reason::OriginUnproven,
                    ))
                })?;
                let readback = self
                    .observations
                    .readbacks
                    .last()
                    .expect("action readback retained");
                let source = ScopedSource {
                    source: session.source.as_ref().expect("retained source chain"),
                    invocation: self.invocation,
                    expected: self.expected,
                    captured: self.captured,
                };
                let checked = if session.recommendation_required {
                    verify_database_readback(&source, readback, &slot, date, cohort)
                } else {
                    verify_database_admission_readback(&source, readback, &slot, date, cohort)
                };
                let checked = match checked {
                    Ok(checked) => checked,
                    Err(error) => return Err(self.fail_first(error)),
                };
                let same_database_identity = if session.recommendation_required {
                    Some(&checked.readback_sha256) == session.database_readback_sha256.as_ref()
                } else {
                    Some(&checked.admission_projection_sha256)
                        == session.database_admission_projection_sha256.as_ref()
                };
                if !same_database_identity {
                    return Err(self.fail(financial::PaperInputActionError::Input(
                        Reason::CohortChanged,
                    )));
                }
                self.checkpoint_fence(fence).await?;
                if let Some(expected_input) = &session.financial_input {
                    // The reference belongs to the retained session, not self's
                    // mutable observation buffers.
                    let readback = self
                        .observations
                        .readbacks
                        .last()
                        .expect("action readback retained");
                    let transaction = self
                        .transaction
                        .as_mut()
                        .expect("active economic transaction");
                    let actual = read_financial_job_input(
                        transaction,
                        readback.as_untrusted_value(),
                        cohort,
                        &mut self.retained_values,
                    )
                    .await;
                    match actual {
                        Ok(actual) if &actual == expected_input => {}
                        Ok(_) => {
                            return Err(self.fail(financial::PaperInputActionError::Input(
                                Reason::CohortChanged,
                            )));
                        }
                        Err(error) => return Err(self.fail_first(error)),
                    }
                    self.checkpoint_fence(fence).await?;
                }
                self.clock
                    .checkpoint()
                    .map_err(|reason| self.fail(financial::PaperInputActionError::Input(reason)))?;
                self.current
                    .revalidate()
                    .map_err(|error| self.fail_first(error))?;
                self.checkpoint_fence(fence).await?;
            }
            self.check_files_with_fence(fence).await
        }

        async fn commit_with_fence(
            &mut self,
            final_check: &mut dyn FnMut() -> Result<(), financial::PaperInputActionError>,
            mut fence: Option<&mut dyn financial::PaperActionCommitFence>,
        ) -> Result<(), financial::PaperInputActionError> {
            self.readback_with_fence(&mut fence).await?;
            if let Err(error) = final_check() {
                return Err(self.fail(error));
            }
            self.check_action_date()?;
            if self.failures.dirty || self.phase != ActionTransactionPhase::Active {
                return Err(self
                    .failures
                    .first
                    .unwrap_or(financial::PaperInputActionError::TransactionState));
            }
            // This fresh checkpoint follows ALL potentially long caller checks.
            self.checkpoint_fence(&mut fence).await?;
            if let Some(fence) = fence {
                let settled = fence
                    .settle(self.transaction.as_mut().expect("active transaction"))
                    .await;
                financial::retain_settlement(&mut self.retained_values, settled)
                    .map_err(|error| self.fail(error))?;
                if let Err(error) = fence
                    .after_settlement(self.transaction.as_mut().expect("active transaction"))
                    .await
                {
                    self.retained_values
                        .push(financial::RetainedActionValue::QueueError(error));
                    return Err(self.fail(financial::PaperInputActionError::Runtime));
                }
            }
            let clock = &mut *self.clock;
            let action_date = self.action_started_seoul_date;
            let recovery = self.calculation_request.worker_recovery.as_ref();
            let transaction = take_after_commit_boundary(
                self.outer_revalidate,
                || {
                    checkpoint_action_date(clock, action_date)?;
                    // SQL checked its own clock under the retained row locks.
                    // Check the same immutable expiry again after caller/fence
                    // work, immediately before the existing COMMIT boundary.
                    if recovery.is_some_and(|selected| !selected.unexpired_at(Utc::now())) {
                        return Err(financial::PaperInputActionError::Runtime);
                    }
                    Ok(())
                },
                &mut self.phase,
                &mut self.transaction,
                &mut self.failures,
            )?;
            match transaction.commit().await {
                Ok(()) => {
                    self.commit_succeeded = true;
                    self.phase = ActionTransactionPhase::Committed;
                }
                Err(error) => {
                    self.retained_values
                        .push(financial::RetainedActionValue::SqlError(error));
                    self.phase = ActionTransactionPhase::CommitUnknown;
                    return Err(self.fail(financial::PaperInputActionError::CommitUnknown));
                }
            }
            if self.check_files().is_err() {
                self.phase = ActionTransactionPhase::CommitUnknown;
                return Err(self.fail(financial::PaperInputActionError::CommitUnknown));
            }
            Ok(())
        }

        async fn finish_scope(&mut self) {
            let unfinished = self.phase == ActionTransactionPhase::Active;
            if self.transaction.is_some() {
                let _ = self.rollback().await;
            }
            if unfinished && self.phase == ActionTransactionPhase::RolledBack {
                self.fail(financial::PaperInputActionError::TransactionState);
            }
            if let Err(error) = self.check_files() {
                self.fail(error);
            }
            if matches!(
                self.phase,
                ActionTransactionPhase::CommitStarted | ActionTransactionPhase::CommitUnknown
            ) && !self
                .failures
                .first
                .is_some_and(|error| error == financial::PaperInputActionError::CommitUnknown)
            {
                self.fail(financial::PaperInputActionError::CommitUnknown);
            }
        }

        fn into_completion(self) -> ActionDriverCompletion {
            ActionDriverCompletion {
                first: self.failures.first,
                secondary: self.failures.secondary,
                committed: self.commit_succeeded,
                commit_started: matches!(
                    self.phase,
                    ActionTransactionPhase::CommitStarted
                        | ActionTransactionPhase::Committed
                        | ActionTransactionPhase::CommitUnknown
                ),
                retained: self.retained_values,
            }
        }
    }

    struct ActionDriverCompletion {
        first: Option<financial::PaperInputActionError>,
        secondary: Vec<financial::PaperInputActionError>,
        committed: bool,
        commit_started: bool,
        retained: Vec<financial::RetainedActionValue>,
    }

    impl financial::ActionInputDriver for ScopedActionDriver<'_, '_, '_, '_, '_, '_> {
        fn revalidate_files(&mut self) -> Result<(), financial::PaperInputActionError> {
            if self.failures.dirty {
                return Err(self
                    .failures
                    .first
                    .unwrap_or(financial::PaperInputActionError::Cleanup));
            }
            match self.check_files() {
                Ok(()) => Ok(()),
                Err(error) => Err(self.fail(error)),
            }
        }

        fn retain_value(&mut self, value: Value) {
            self.retained_values
                .push(financial::RetainedActionValue::SqlValue(value));
        }

        fn retain_sql_error(&mut self, error: sqlx::Error) {
            self.retained_values
                .push(financial::RetainedActionValue::SqlError(error));
        }

        fn retain_queue_error(&mut self, error: crate::QueueError) {
            self.retained_values
                .push(financial::RetainedActionValue::QueueError(error));
        }

        fn commit_started(&self) -> bool {
            matches!(
                self.phase,
                ActionTransactionPhase::CommitStarted
                    | ActionTransactionPhase::Committed
                    | ActionTransactionPhase::CommitUnknown
            )
        }

        fn begin_transaction<'a>(
            &'a mut self,
        ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
            Box::pin(async move {
                if self.phase != ActionTransactionPhase::NotStarted {
                    return Err(self.fail(financial::PaperInputActionError::TransactionState));
                }
                self.revalidate_files()?;
                let transaction = match self.pool.begin().await {
                    Ok(transaction) => transaction,
                    Err(error) => {
                        self.retain_sql_error(error);
                        return Err(self.fail(financial::PaperInputActionError::Sql));
                    }
                };
                self.transaction = Some(transaction);
                self.phase = ActionTransactionPhase::Active;
                let setup = crate::paper_execution::set_paper_transaction_timeouts(
                    self.transaction.as_mut().expect("begun transaction"),
                )
                .await;
                if let Err(error) = setup {
                    self.retain_sql_error(error);
                    return Err(self.fail(financial::PaperInputActionError::Sql));
                }
                if self.calculation_request.worker_recovery.is_some() {
                    // Acquire the exact grant/current fences before any consumer
                    // can issue economics; keep them through final COMMIT.
                    self.readback_on_action_transaction().await?;
                }
                self.revalidate_files()
            })
        }

        fn transaction(
            &mut self,
        ) -> Result<&mut Transaction<'static, Postgres>, financial::PaperInputActionError> {
            if self.phase != ActionTransactionPhase::Active || self.failures.dirty {
                return Err(self.fail(financial::PaperInputActionError::TransactionState));
            }
            if self.transaction.is_none() {
                return Err(self.fail(financial::PaperInputActionError::TransactionState));
            }
            Ok(self
                .transaction
                .as_mut()
                .expect("checked active transaction"))
        }

        fn revalidate_before_commit<'a>(
            &'a mut self,
        ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
            Box::pin(async move { self.readback_on_action_transaction().await })
        }

        fn commit<'a>(
            &'a mut self,
            final_check: &'a mut dyn FnMut() -> Result<(), financial::PaperInputActionError>,
        ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
            Box::pin(async move { self.commit_with_fence(final_check, None).await })
        }

        fn commit_fenced<'a>(
            &'a mut self,
            final_check: &'a mut dyn FnMut() -> Result<(), financial::PaperInputActionError>,
            fence: &'a mut dyn financial::PaperActionCommitFence,
        ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
            Box::pin(async move { self.commit_with_fence(final_check, Some(fence)).await })
        }

        fn rollback<'a>(
            &'a mut self,
        ) -> financial::LocalInputFuture<'a, Result<(), financial::PaperInputActionError>> {
            Box::pin(async move {
                if self.phase == ActionTransactionPhase::CommitStarted
                    || self.phase == ActionTransactionPhase::Committed
                    || self.phase == ActionTransactionPhase::CommitUnknown
                {
                    return Ok(());
                }
                let Some(transaction) = self.transaction.take() else {
                    if self.phase == ActionTransactionPhase::Active {
                        self.phase = ActionTransactionPhase::RolledBack;
                    }
                    return Ok(());
                };
                match transaction.rollback().await {
                    Ok(()) => {
                        self.phase = ActionTransactionPhase::RolledBack;
                        Ok(())
                    }
                    Err(error) => {
                        self.retained_values
                            .push(financial::RetainedActionValue::SqlError(error));
                        self.phase = ActionTransactionPhase::CommitUnknown;
                        Err(self.fail(financial::PaperInputActionError::CommitUnknown))
                    }
                }
            })
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Genuine borrowed holders, actual scope endpoints and retained results remain explicit."
    )]
    async fn run_sessions<'env, 'i>(
        pool: &PgPool,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        current: &Current<'_, '_, '_>,
        clock: &mut EvaluationClock,
        context: &'env DailyContext,
        config: &mut ConfigSession<'i, 'i>,
        retained: &mut ContextLoadRetention,
        raw: &Raw<'i>,
        snapshots: &CheckedRawEvidenceSnapshots<'_, 'i, 'i>,
        root: &'env CheckedDailyAdmissionRoot<'i, 'i>,
        historical: &'env CheckedHistoricalApproval<'i>,
        dates: Vec<NaiveDate>,
        capture_request: Option<&financial::PaperCalculationInputRequest>,
        action_request: Option<&financial::PaperActionInputRequest>,
        outer_revalidate: &mut Option<&mut dyn financial::ActionContextGuard>,
        action_consumer: &mut Option<&mut dyn financial::LocalActionConsumer>,
        evaluated_at: DateTime<Utc>,
        eligible_date: NaiveDate,
        expires_at: DateTime<Utc>,
        observations: &mut Observations,
    ) -> Result<Option<PaperEligibilityReport>, First> {
        let mut transaction = None;
        let mut sessions = Vec::with_capacity(MAX_SESSIONS);
        let mut session_dates = dates.clone();
        if let Some(request) = capture_request {
            for requested in request.price_dates() {
                if !session_dates.contains(&requested) {
                    session_dates.push(requested);
                }
            }
        }
        let original = async {
            clock.checkpoint()?;
            current.revalidate()?;
            transaction = Some(pool.begin().await.map_err(First::sql_setup)?);
            clock.checkpoint()?;
            current.revalidate()?;
            sqlx::raw_sql(CHECKER_TRANSACTION_SETUP)
                .execute(&mut **transaction.as_mut().expect("tx1"))
                .await
                .map_err(First::sql_setup)?;
            crate::paper_execution::set_paper_transaction_timeouts(
                transaction.as_mut().expect("tx1"),
            )
            .await
            .map_err(First::sql_setup)?;
            current.revalidate()?;
            for date in session_order(&session_dates).copied() {
                current.revalidate()?;
                // Retain even the partially acquired current session before any fallible use.
                sessions.push(Session::new(
                    date,
                    capture_request.map_or(current.cohort(), |request| {
                        request.cohort_for(date, current.cohort())
                    }),
                    capture_request.is_some(),
                    capture_request.is_none_or(|request| !request.is_price_only(date)),
                    capture_request.and_then(|request| request.expected_run_id(date)),
                ));
                read_session(
                    transaction.as_mut().expect("tx1"),
                    recovery_reader,
                    capture_request.and_then(|request| request.worker_recovery.as_ref()),
                    context,
                    snapshots,
                    root,
                    historical,
                    sessions.last_mut().expect("pending session"),
                    invocation,
                    expected,
                    captured,
                    clock,
                    current,
                    observations,
                )
                .await?;
                clock.checkpoint()?;
                current.revalidate()?;
            }
            clock.checkpoint()?;
            current.revalidate()?;
            take_for_commit(&mut transaction)
                .commit()
                .await
                .map_err(First::sql_setup)?;
            clock.checkpoint()?;
            current.revalidate()?;
            revalidate_retained_artifacts(root, historical, expected, captured)?;
            for session in &sessions {
                clock.checkpoint()?;
                current.revalidate()?;
                session.revalidate(invocation, expected, captured)?;
                current.revalidate()?;
            }
            context
                .revalidate_in(&mut *config, retained, current.expected)
                .map_err(|e| {
                    First::new(
                        e.reason().copied().unwrap_or(Reason::ReaderIncompatible),
                        Source::Config(Box::new(e)),
                    )
                })?;
            current.revalidate()?;
            let references = sessions
                .iter()
                .flat_map(|session| session.raw_references.iter().cloned())
                .collect::<Vec<_>>();
            raw.refresh(
                context,
                clock,
                &dates,
                &references,
                capture_request.and_then(|request| request.calendar_range()),
                action_request,
                observations,
            )?;
            current.revalidate()?;
            if let Some(action_selector) = action_request {
                let Some(calculation_request) = capture_request else {
                    return Err(Reason::ReaderIncompatible.into());
                };
                let Some(outer_check) = outer_revalidate.as_deref_mut() else {
                    return Err(Reason::ReaderIncompatible.into());
                };
                let range_proof = observations.calendar_range_proof.take();
                let inputs = checked_action_inputs(
                    current.expected,
                    action_selector,
                    calculation_request,
                    &dates,
                    &sessions,
                    range_proof.as_ref(),
                    evaluated_at,
                    expires_at,
                )?;
                let action_started_seoul_date = clock
                    .started_at
                    .with_timezone(
                        &FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul civil offset"),
                    )
                    .date_naive();
                let references = sessions
                    .iter()
                    .flat_map(|session| session.raw_references.iter().cloned())
                    .collect::<Vec<_>>();
                let mut driver = ScopedActionDriver {
                    pool,
                    recovery_reader,
                    invocation,
                    expected,
                    captured,
                    current,
                    clock,
                    context,
                    config,
                    retained,
                    raw,
                    root,
                    historical,
                    sessions: &sessions,
                    dates: &dates,
                    references: &references,
                    calculation_request,
                    action_request: action_selector,
                    range_proof: range_proof.as_ref(),
                    action_started_seoul_date,
                    outer_revalidate: outer_check,
                    observations,
                    transaction: None,
                    phase: ActionTransactionPhase::NotStarted,
                    commit_succeeded: false,
                    failures: ActionFailures::default(),
                    retained_values: Vec::new(),
                };
                let callback_result = {
                    let mut checked = financial::CheckedPaperAction::new(&inputs, &mut driver);
                    match action_consumer.as_deref_mut() {
                        Some(consumer) => consumer.invoke(&mut checked).await,
                        None => Err(financial::PaperInputActionError::TransactionState),
                    }
                };
                if let Err(error) = callback_result {
                    driver.fail(error);
                }
                driver.finish_scope().await;
                let completion = driver.into_completion();
                observations.clean &= completion.first.is_none() && completion.secondary.is_empty();
                observations.action_first = completion.first;
                observations.action_secondary = completion.secondary;
                observations.action_committed = completion.committed;
                observations.action_commit_started = completion.commit_started;
                observations.action_retained.extend(completion.retained);
                observations.calendar_range_proof = range_proof;
                clock.checkpoint()?;
                current.revalidate()?;
                return Ok(None);
            }
            revalidate_database(
                pool,
                recovery_reader,
                capture_request.and_then(|request| request.worker_recovery.as_ref()),
                &mut transaction,
                &sessions,
                invocation,
                expected,
                captured,
                clock,
                current,
                observations,
            )
            .await?;
            revalidate_retained_artifacts(root, historical, expected, captured)?;
            for session in &sessions {
                clock.checkpoint()?;
                current.revalidate()?;
                session.revalidate(invocation, expected, captured)?;
                current.revalidate()?;
            }
            context
                .revalidate_in(&mut *config, retained, current.expected)
                .map_err(|e| {
                    First::new(
                        e.reason().copied().unwrap_or(Reason::ReaderIncompatible),
                        Source::Config(Box::new(e)),
                    )
                })?;
            current.revalidate()?;
            raw.refresh(
                context,
                clock,
                &dates,
                &references,
                capture_request.and_then(|request| request.calendar_range()),
                action_request,
                observations,
            )?;
            current.revalidate()?;
            revalidate_retained_artifacts(root, historical, expected, captured)?;
            let finished_at = clock.checkpoint()?;
            current.revalidate()?;
            if discover(
                raw,
                context,
                finished_at,
                capture_request.and_then(|request| request.calendar_range()),
                action_request,
                observations,
            )? != dates
            {
                return Err(Reason::CalendarUnproven.into());
            }
            clock.checkpoint()?;
            current.revalidate()?;
            let facts = dates
                .iter()
                .map(|date| {
                    sessions
                        .iter()
                        .find(|session| session.date == *date)
                        .and_then(|session| session.facts.as_ref())
                        .cloned()
                        .ok_or(Reason::EvidenceMissing)
                })
                .collect::<Vec<Result<VerifiedSessionFacts, Reason>>>();
            let report = evaluate_verified_sessions(
                &facts,
                current.cohort(),
                &dates,
                eligible_date,
                evaluated_at,
                expires_at,
            );
            clock.checkpoint()?;
            current.revalidate()?;
            clock.checkpoint()?;
            Ok(Some(report))
        }
        .await;
        rollback_owned(&mut transaction, observations).await;
        drain_sessions(&mut sessions, |session| session.finish(observations));
        original
    }
    struct ScopedSource<'v, 'i> {
        source: &'v CheckedProtectedSourceChain<'i, 'i>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
    }
    impl ScopedSource<'_, '_> {
        fn check(&self) -> Result<(), First> {
            self.source
                .check_scope_in(self.invocation, self.expected, self.captured)
                .map_err(|e| First::new(Reason::EvidenceTampered, Source::Chain(Box::new(e))))
        }
    }
    impl CheckerSourcePolicy for ScopedSource<'_, '_> {
        type Error = First;
        fn policy_checked_arm(&self) -> Result<Option<CheckedArmOrigin>, First> {
            self.check()?;
            self.source
                .revalidate()
                .map_err(|e| First::new(Reason::EvidenceTampered, Source::Chain(Box::new(e))))?;
            let first = self.policy_first()?;
            let intent = self.policy_event("ARM_INTENT", Reason::OriginUnproven)?;
            let armed = self.policy_event("ARMED", Reason::OriginUnproven)?;
            let reserved = self.policy_event("RESERVED", Reason::OriginUnproven)?;
            // The protected reader validated the whole predecessor/clock chain.
            // Recheck inherited source/parent and sequence identities at the join.
            for event in [intent, armed, reserved] {
                for key in [
                    "slot_id",
                    "session",
                    "cohort",
                    "reservation_id",
                    "arm",
                    "invocation_id",
                ] {
                    if event[key] != first[key] {
                        return Err(Reason::OriginUnproven.into());
                    }
                }
            }
            let intent_seq = intent["sequence"].as_i64().ok_or(Reason::OriginUnproven)?;
            let armed_seq = armed["sequence"].as_i64().ok_or(Reason::OriginUnproven)?;
            let reserved_seq = reserved["sequence"]
                .as_i64()
                .ok_or(Reason::OriginUnproven)?;
            let arm_intent_sha256 = canonical_source_digest(intent)?;
            if intent_seq.checked_add(1) != Some(armed_seq)
                || armed_seq >= reserved_seq
                || armed["previous_sha256"] != arm_intent_sha256
            {
                return Err(Reason::OriginUnproven.into());
            }
            let arm_intent_observed = ClockTriple::from_value(&intent["observed"])?;
            let armed_observed = ClockTriple::from_value(&armed["observed"])?;
            Ok(Some(CheckedArmOrigin {
                boot_id: checked_source_uuid(&first["arm"]["boot_id"])?,
                generation: checked_source_uuid(&first["arm"]["generation"])?,
                nonce: checked_source_uuid(&first["arm"]["nonce"])?,
                invocation_id: checked_source_uuid(&first["invocation_id"])?,
                arm_intent_sha256,
                armed_sha256: canonical_source_digest(armed)?,
                arm_intent_observed,
                armed_observed,
            }))
        }
        fn policy_first(&self) -> Result<&Value, First> {
            self.check()?;
            self.source
                .first()
                .map_err(|e| First::new(Reason::OriginUnproven, Source::Chain(Box::new(e))))
        }
        fn policy_event(&self, kind: &str, reason: Reason) -> Result<&Value, First> {
            self.check()?;
            self.source
                .event(kind)
                .map_err(|e| First::new(reason, Source::Chain(Box::new(e))))
        }
        fn policy_events(&self) -> Result<&[Value], First> {
            self.check()?;
            self.source
                .events()
                .map_err(|e| First::new(Reason::OriginUnproven, Source::Chain(Box::new(e))))
        }
    }
    struct RawPolicy<'v, 's, 'i> {
        snapshots: &'v CheckedRawEvidenceSnapshots<'s, 'i, 'i>,
        observations: &'v mut Observations,
    }
    impl CheckerRawPolicy for RawPolicy<'_, '_, '_> {
        type Error = First;
        fn matches_original(
            &mut self,
            reference: &SourceFileReference,
            evidence: &DailyFileEvidenceV1Wire,
        ) -> Result<bool, First> {
            let resolved = resolve_original_checked(self.snapshots, reference)
                .map_err(|e| First::new(Reason::EvidenceTampered, Source::Lineage(Box::new(e))))?;
            let result = match resolved.view_in() {
                Ok(view) => {
                    let equal = view.evidence() == evidence;
                    self.observations.local(view.into_diagnostics());
                    Ok(equal)
                }
                Err(e) => Err(First::new(
                    Reason::EvidenceTampered,
                    Source::Lineage(Box::new(e)),
                )),
            };
            self.observations
                .lineage
                .push(resolved.retain_diagnostics());
            result
        }
    }
    fn retain_replay<T, E>(
        result: Result<T, E>,
        callback: Option<E>,
        post: Result<(), E>,
        slot: &mut Option<T>,
        later: &mut Vec<E>,
    ) -> Result<(), E> {
        let lower = match result {
            Ok(value) => {
                *slot = Some(value);
                None
            }
            Err(e) => Some(e),
        };
        if let Some(first) = callback {
            if let Some(e) = lower {
                later.push(e);
            }
            if let Err(e) = post {
                later.push(e);
            }
            return Err(first);
        }
        if let Some(first) = lower {
            if let Err(e) = post {
                later.push(e);
            }
            return Err(first);
        }
        post
    }
    fn retain_acquired<T, E>(result: Result<T, E>, slot: &mut Option<T>) -> Result<(), E> {
        *slot = Some(result?);
        Ok(())
    }
    fn finish_session_resources<C, S>(
        chain: Option<C>,
        source: Option<S>,
        chain_finish: impl FnOnce(C),
        source_finish: impl FnOnce(S),
    ) {
        if let Some(chain) = chain {
            chain_finish(chain);
        }
        if let Some(source) = source {
            source_finish(source);
        }
    }
    fn session_order<T>(sessions: &[T]) -> std::slice::Iter<'_, T> {
        sessions.iter()
    }
    fn drain_sessions<T>(sessions: &mut Vec<T>, mut finish: impl FnMut(T)) {
        for session in std::mem::take(sessions) {
            finish(session);
        }
    }
    async fn read_session_database(
        transaction: &mut Transaction<'_, Postgres>,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        worker_recovery: Option<&financial::WorkerRecoverySelection>,
        date: NaiveDate,
        cohort: &Value,
        receipt: Option<&str>,
    ) -> Result<Value, sqlx::Error> {
        if let Some(worker) = worker_recovery {
            if recovery_reader.is_some() {
                return Err(sqlx::Error::Protocol("mixed recovery reader".into()));
            }
            // The independent current three-session eligibility window keeps
            // the ordinary current-only reader; only original T/E use the grant.
            if worker.selects(date) {
                return worker.read(transaction, date, cohort, receipt).await;
            }
        }
        match recovery_reader {
            Some(reader) => reader.read(date, cohort).await,
            None => {
                sqlx::query_scalar("SELECT public.read_owner_beta_paper_session_evidence($1,$2)")
                    .bind(date)
                    .bind(cohort)
                    .fetch_one(&mut **transaction)
                    .await
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Genuine borrowed holders, actual scope endpoints and retained results remain explicit."
    )]
    async fn read_session<'env, 'i>(
        transaction: &mut Transaction<'_, Postgres>,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        worker_recovery: Option<&financial::WorkerRecoverySelection>,
        context: &'env DailyContext,
        snapshots: &CheckedRawEvidenceSnapshots<'_, 'i, 'i>,
        root: &'env CheckedDailyAdmissionRoot<'i, 'i>,
        historical: &'env CheckedHistoricalApproval<'i>,
        session: &mut Session<'env, 'i>,
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        clock: &mut EvaluationClock,
        current: &Current<'_, '_, '_>,
        observations: &mut Observations,
    ) -> Result<(), First> {
        let original_cohort = session.cohort.clone();
        let cohort = &original_cohort;
        let date = session.date;
        let date_text = date.to_string();
        let trading_date = TradingDate::parse(&date_text).map_err(|_| Reason::CalendarUnproven)?;
        let slot_id = wire::slot_id(cohort, &date_text).map_err(|_| Reason::OriginUnproven)?;
        current.revalidate()?;
        retain_acquired(
            ProtectedSourceChain::open_checked_in(
                Path::new(SOURCE_EVENTS_ROOT),
                &slot_id,
                invocation,
                expected,
                captured,
            )
            .map_err(|e| First::new(Reason::OriginUnproven, Source::Chain(Box::new(e)))),
            &mut session.source,
        )?;
        let source = ScopedSource {
            source: session.source.as_ref().expect("actual Source retained"),
            invocation,
            expected,
            captured,
        };
        source.check()?;
        source
            .source
            .revalidate()
            .map_err(|e| First::new(Reason::EvidenceTampered, Source::Chain(Box::new(e))))?;
        current.revalidate()?;
        if source.policy_first()?["session"] != date_text
            || source.policy_first()?["cohort"] != *cohort
            || source.policy_first()?["slot_id"] != slot_id
        {
            return Err(Reason::OriginUnproven.into());
        }
        validate_source_admission_state(source.policy_events()?, session.recommendation_required)?;
        clock.checkpoint()?;
        current.revalidate()?;
        let value = read_session_database(
            transaction,
            recovery_reader,
            worker_recovery,
            date,
            cohort,
            None,
        )
        .await
        .map_err(First::sql)?;
        observations.values.push(value);
        clock.checkpoint()?;
        current.revalidate()?;
        let readback = UntrustedPaperSessionReadbackV1::from_value(
            observations
                .values
                .last()
                .expect("returned SQL Value")
                .clone(),
        )
        .map_err(|_| Reason::EvidenceTampered)?;
        observations.readbacks.push(readback);
        let database = if session.recommendation_required {
            verify_database_readback(
                &source,
                observations.readbacks.last().expect("actual readback"),
                &slot_id,
                date,
                cohort,
            )?
        } else {
            verify_database_admission_readback(
                &source,
                observations.readbacks.last().expect("actual readback"),
                &slot_id,
                date,
                cohort,
            )?
        };
        session.database_readback_sha256 = Some(database.readback_sha256.clone());
        session.database_admission_projection_sha256 =
            Some(database.admission_projection_sha256.clone());
        session.native_enable_session = database.native_enable_session;
        session.pins = Some(
            serde_json::from_value(database.pins.clone()).map_err(|_| Reason::InputPinMismatch)?,
        );
        let pins = session.pins.as_ref().expect("actual parsed pins");
        if pins.policy_sha256 != context.policy_hash {
            return Err(Reason::InputPinMismatch.into());
        }
        current.revalidate()?;
        historical
            .approved_in(expected, captured)
            .map(|_| ())
            .map_err(|e| {
                First::new(Reason::InputPinMismatch, Source::HistoricalUse(Box::new(e)))
            })?;
        historical
            .revalidate_in(expected, captured)
            .map_err(|e| First::new(Reason::InputPinMismatch, Source::Historical(Box::new(e))))?;
        let mut callback = None;
        let chain_result = read_sealed_daily_chain_checked(
            invocation,
            expected,
            captured,
            root,
            pins,
            historical,
            context.config.policy(),
            context.config.replay_limits(),
            || match clock.checkpoint() {
                Ok(_) => true,
                Err(reason) => {
                    if callback.is_none() {
                        callback = Some(First::from(reason));
                    }
                    false
                }
            },
        )
        .map_err(|e| First::new(Reason::InputPinMismatch, Source::Daily(Box::new(e))));
        let lower = match chain_result {
            Ok(chain) => {
                session.chain = Some(chain);
                Ok(())
            }
            Err(e) => Err(e),
        };
        let post = current.revalidate();
        retain_replay(
            lower,
            callback,
            post,
            &mut None,
            &mut observations.secondary,
        )?;
        let chain = session
            .chain
            .as_ref()
            .expect("actual chain retained before postcheck");
        let mut financial_snapshot = None;
        let mut financial_observed_at = None;
        let mut financial_availability = None;
        let mut financial_input = None;
        let mut financial_target = None;
        let price_available = {
            let admitted = chain
                .admitted(expected, captured)
                .map_err(|e| First::new(Reason::InputPinMismatch, Source::DailyUse(Box::new(e))))?;
            let view = admitted
                .view_in(expected, captured)
                .map_err(|e| First::new(Reason::InputPinMismatch, Source::DailyUse(Box::new(e))))?;
            // The existing Legacy as_of getter is manifest.range_end; keep both original predicates.
            let as_of = view.manifest().range_end;
            if as_of != trading_date
                || view.manifest().range_end != trading_date
                || view.pins() != pins
            {
                return Err(Reason::InputPinMismatch.into());
            }
            let daily = daily_session_for_date(view.manifest(), trading_date)
                .ok_or(Reason::InputNotAdmitted)?;
            if target_calendar_binding(daily, trading_date).is_none() {
                return Err(Reason::CalendarUnproven.into());
            }
            if session.capture_financial {
                financial_snapshot = Some(
                    financial::PaperPriceSnapshotV1::from_checked_view(&view)
                        .map_err(|_| Reason::InputPinMismatch)?,
                );
                let readback = observations
                    .readbacks
                    .last()
                    .expect("validated source readback");
                if session.recommendation_required {
                    let input = read_financial_job_input(
                        transaction,
                        readback.as_untrusted_value(),
                        cohort,
                        &mut observations.action_retained,
                    )
                    .await?;
                    if session
                        .expected_run_id
                        .is_some_and(|expected_run_id| input.run_id() != expected_run_id)
                    {
                        return Err(Reason::CohortChanged.into());
                    }
                    clock.checkpoint()?;
                    current.revalidate()?;
                    let computation =
                        crate::owner_beta::daily_compute::compute_owner_beta_daily_recommendation_checked(
                            &view, &input,
                        )
                        .map_err(|_| Reason::InputPinMismatch)?;
                    if readback.as_untrusted_value()["factor_snapshot_sha256"].as_str()
                        != Some(computation.factor_snapshot().hash.as_str())
                        || readback.as_untrusted_value()["target_snapshot_sha256"].as_str()
                            != Some(
                                computation
                                    .target_snapshot()
                                    .target_snapshot_sha256()
                                    .as_str(),
                            )
                    {
                        return Err(Reason::InputPinMismatch.into());
                    }
                    financial_target = Some(computation.target_snapshot().clone());
                    financial_input = Some(input);
                }
                let observed_at = clock.checkpoint()?;
                // Raw proof below validates these exact file identities and original times.
                // Retain the latest contributing price/reference original retrieval, not
                // the later local copy time or an invented exchange-open timestamp.
                let original_at = daily
                    .bars
                    .iter()
                    .chain(&daily.reference)
                    .map(|file| file.original_retrieved_at.as_timestamp().as_datetime())
                    .max()
                    .ok_or(Reason::PriceNotAvailable)?;
                let admitted_at = readback.as_untrusted_value()["admitted_at"]
                    .as_str()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc))
                    .ok_or(Reason::InputNotAdmitted)?;
                if original_at > observed_at || admitted_at > observed_at {
                    return Err(Reason::PriceNotAvailable.into());
                }
                financial_availability = Some(financial::PriceAvailabilityObservation {
                    source_original_retrieved_at: original_at,
                    input_admitted_at: admitted_at,
                    usable_at: observed_at,
                });
                financial_observed_at = Some(observed_at);
                current.revalidate()?;
            }
            let raw_evidence = verify_daily_raw(
                &source,
                daily,
                &mut RawPolicy {
                    snapshots,
                    observations,
                },
                trading_date,
            )?;
            let price_available = !daily.bars.is_empty() && !daily.reference.is_empty();
            retain_before_postcheck(&mut session.raw_aggregates, raw_evidence, |_| {
                admitted.revalidate_in(expected, captured).map_err(|e| {
                    First::new(Reason::InputPinMismatch, Source::DailyUse(Box::new(e)))
                })
            })?;
            price_available
        };
        if session
            .raw_aggregates
            .last()
            .expect("actual Raw aggregate retained before postcheck")
            .original_identities
            != database.original_identities
        {
            return Err(Reason::EvidenceTampered.into());
        }
        validate_checked_input_in(chain, date, pins, expected, captured)
            .map_err(|e| First::new(Reason::InputPinMismatch, Source::Input(Box::new(e))))?;
        current.revalidate()?;
        if session.capture_financial {
            session.financial_snapshot = financial_snapshot;
            session.financial_readback = Some(
                observations
                    .readbacks
                    .last()
                    .expect("first validated SQL readback retained")
                    .clone(),
            );
            session.financial_observed_at = financial_observed_at;
            session.financial_availability = financial_availability;
            session.financial_input = financial_input;
            session.financial_target = financial_target;
        }
        let raw_evidence = session
            .raw_aggregates
            .pop()
            .expect("actual Raw aggregate retained through all postchecks");
        let original_identities = raw_evidence.original_identities;
        session.raw_references = raw_evidence.references;
        session.facts = Some(VerifiedSessionFacts {
            date,
            cohort: session.cohort.clone(),
            source_identities: database.source_identities,
            run_identities: database.run_identities,
            original_identities,
            mode: SessionMode::Scheduled,
            terminal_succeeded: session.recommendation_required,
            rights_active: database.current_rights_verified,
            rights_sufficient: database.current_rights_verified,
            source_revoked: !database.current_admission_verified,
            run_linked: session.recommendation_required,
            input_admitted: true,
            pins_match: true,
            price_available,
            replayed: false,
            revalidated: true,
            daily_hashes: [
                pins.content_sha256.as_str().to_owned(),
                pins.manifest_sha256.as_str().to_owned(),
                pins.admission_sha256.as_str().to_owned(),
                pins.policy_sha256.as_str().to_owned(),
            ],
        });
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Genuine borrowed holders, actual scope endpoints and retained results remain explicit."
    )]
    async fn revalidate_database<'i>(
        pool: &PgPool,
        recovery_reader: Option<&NativeRecoveryReadContext>,
        worker_recovery: Option<&financial::WorkerRecoverySelection>,
        transaction: &mut Option<Transaction<'static, Postgres>>,
        sessions: &[Session<'_, 'i>],
        invocation: &'i RawInvocation<'i>,
        expected: &'i Attempt,
        captured: &'i Attempt,
        clock: &mut EvaluationClock,
        current: &Current<'_, '_, '_>,
        observations: &mut Observations,
    ) -> Result<(), First> {
        clock.checkpoint()?;
        current.revalidate()?;
        *transaction = Some(pool.begin().await.map_err(First::sql_setup)?);
        clock.checkpoint()?;
        current.revalidate()?;
        sqlx::raw_sql(CHECKER_TRANSACTION_SETUP)
            .execute(&mut **transaction.as_mut().expect("tx2"))
            .await
            .map_err(First::sql_setup)?;
        crate::paper_execution::set_paper_transaction_timeouts(transaction.as_mut().expect("tx2"))
            .await
            .map_err(First::sql_setup)?;
        current.revalidate()?;
        for session in session_order(sessions) {
            let cohort = &session.cohort;
            clock.checkpoint()?;
            current.revalidate()?;
            let value = read_session_database(
                transaction.as_mut().expect("tx2"),
                recovery_reader,
                worker_recovery,
                session.date,
                cohort,
                None,
            )
            .await
            .map_err(First::sql)?;
            observations.values.push(value);
            current.revalidate()?;
            let readback = UntrustedPaperSessionReadbackV1::from_value(
                observations
                    .values
                    .last()
                    .expect("returned SQL Value")
                    .clone(),
            )
            .map_err(|_| Reason::EvidenceTampered)?;
            observations.readbacks.push(readback);
            let source = ScopedSource {
                source: session.source.as_ref().expect("retained Source"),
                invocation,
                expected,
                captured,
            };
            let checked = if session.recommendation_required {
                verify_database_readback(
                    &source,
                    observations.readbacks.last().expect("actual readback"),
                    &wire::slot_id(cohort, &session.date.to_string())
                        .map_err(|_| Reason::OriginUnproven)?,
                    session.date,
                    cohort,
                )?
            } else {
                verify_database_admission_readback(
                    &source,
                    observations.readbacks.last().expect("actual readback"),
                    &wire::slot_id(cohort, &session.date.to_string())
                        .map_err(|_| Reason::OriginUnproven)?,
                    session.date,
                    cohort,
                )?
            };
            let same_database_identity = if session.recommendation_required {
                Some(&checked.readback_sha256) == session.database_readback_sha256.as_ref()
            } else {
                Some(&checked.admission_projection_sha256)
                    == session.database_admission_projection_sha256.as_ref()
            };
            if !same_database_identity {
                return Err(Reason::CohortChanged.into());
            }
            if let Some(expected_input) = &session.financial_input {
                let actual = read_financial_job_input(
                    transaction.as_mut().expect("tx2"),
                    observations
                        .readbacks
                        .last()
                        .expect("second readback")
                        .as_untrusted_value(),
                    cohort,
                    &mut observations.action_retained,
                )
                .await?;
                if &actual != expected_input {
                    return Err(Reason::CohortChanged.into());
                }
            }
            clock.checkpoint()?;
            current.revalidate()?;
        }
        clock.checkpoint()?;
        current.revalidate()?;
        take_for_commit(transaction)
            .commit()
            .await
            .map_err(First::sql_setup)?;
        clock.checkpoint()?;
        current.revalidate()?;
        Ok(())
    }
    async fn read_financial_job_input(
        transaction: &mut Transaction<'_, Postgres>,
        readback: &Value,
        cohort: &Value,
        retained: &mut Vec<financial::RetainedActionValue>,
    ) -> Result<crate::owner_beta::daily_input::OwnerBetaDailyRecommendationInputV1, First> {
        let uuid = |value: &Value, key: &str| -> Result<Uuid, Reason> {
            Uuid::parse_str(string(value, key)?).map_err(|_| Reason::EvidenceTampered)
        };
        let run = uuid(readback, "candidate_run_id")?;
        let job = uuid(readback, "job_id")?;
        let owner = uuid(cohort, "owner_user_id")?;
        let config = uuid(cohort, "strategy_config_id")?;
        let payload: Value = match sqlx::query_scalar::<_, Value>(
            "SELECT j.payload_json FROM public.owner_beta_recommendation_runs r \
             JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id \
             WHERE r.id=$1 AND j.id=$2 AND r.owner_user_id=$3 AND r.strategy_config_id=$4 \
             AND r.status='SUCCEEDED' AND j.status='SUCCEEDED' FOR SHARE OF r,j",
        )
        .bind(run)
        .bind(job)
        .bind(owner)
        .bind(config)
        .fetch_one(&mut **transaction)
        .await
        {
            Ok(payload) => {
                retained.push(financial::RetainedActionValue::SqlValue(payload.clone()));
                payload
            }
            Err(error) => {
                return Err(First::sql(error));
            }
        };
        let input: crate::owner_beta::daily_input::OwnerBetaDailyRecommendationInputV1 =
            serde_json::from_value(payload).map_err(|_| Reason::EvidenceTampered)?;
        if input.run_id() != run
            || input.strategy_config_id() != config
            || Some(input.strategy_snapshot().config_sha256().as_str())
                != cohort["strategy_config_sha256"].as_str()
            || Some(input.config_epoch()) != cohort["config_epoch"].as_i64()
            || Some(input.activation_epoch()) != cohort["activation_epoch"].as_i64()
        {
            return Err(Reason::StrategyBindingMismatch.into());
        }
        Ok(input)
    }
    fn validate_paper_calendar_range(
        proof: &market_data::daily_price_only::calendar::CalendarRangeProof,
        start: TradingDate,
        end: TradingDate,
    ) -> Result<(), Reason> {
        if start >= end {
            return Err(Reason::CalendarUnproven);
        }
        let span = end
            .as_naive_date()
            .signed_duration_since(start.as_naive_date())
            .num_days();
        if span < 1 || span as u64 > MAX_CALENDAR_DAYS {
            return Err(Reason::CalendarUnproven);
        }
        let expected_len = usize::try_from(span + 1).map_err(|_| Reason::CalendarUnproven)?;
        if proof.days().len() != expected_len {
            return Err(Reason::CalendarUnproven);
        }
        for (index, day) in proof.days().iter().enumerate() {
            let expected = start
                .checked_add_days(i64::try_from(index).map_err(|_| Reason::CalendarUnproven)?)
                .map_err(|_| Reason::CalendarUnproven)?;
            let is_endpoint = index == 0 || index + 1 == expected_len;
            if day.date() != expected
                || (day.session_type() == DailySessionTypeV1::Trading) != is_endpoint
                || day.witnesses().is_empty()
            {
                return Err(Reason::CalendarUnproven);
            }
        }
        Ok(())
    }

    fn same_calendar_range_proof(
        left: &market_data::daily_price_only::calendar::CalendarRangeProof,
        right: &market_data::daily_price_only::calendar::CalendarRangeProof,
    ) -> bool {
        left.owner() == right.owner()
            && left.policy_sha256() == right.policy_sha256()
            && left.days().len() == right.days().len()
            && left.days().iter().zip(right.days()).all(|(left, right)| {
                left.date() == right.date()
                    && left.session_type() == right.session_type()
                    && left.witnesses() == right.witnesses()
            })
    }

    fn retain_calendar_range_proof(
        observations: &mut Observations,
        proof: market_data::daily_price_only::calendar::CalendarRangeProof,
    ) -> Result<(), First> {
        if let Some(previous) = observations.calendar_range_proof.as_ref() {
            if !same_calendar_range_proof(previous, &proof) {
                return Err(Reason::CalendarUnproven.into());
            }
        } else {
            observations.calendar_range_proof = Some(proof);
        }
        Ok(())
    }

    fn discover<'i>(
        raw: &Raw<'i>,
        context: &DailyContext,
        now: DateTime<Utc>,
        capture_range: Option<(TradingDate, TradingDate)>,
        action_request: Option<&financial::PaperActionInputRequest>,
        observations: &mut Observations,
    ) -> Result<Vec<NaiveDate>, First> {
        let timestamp = UtcTimestamp::from_datetime(now);
        let snapshots = raw.snapshots(context, timestamp)?;
        let result = (|| {
            let mut references = Vec::new();
            for provider in ["kis", "kis-calendar"] {
                let (view, local) = snapshots
                    .snapshot_observed_view_in(provider, "kr")
                    .map_err(|e| {
                        First::new(Reason::CalendarUnproven, Source::Lineage(Box::new(e)))
                    })?;
                observations.local(local);
                observations.require_clean()?;
                let scope = view.scope().expect("genuine complete checked scope");
                let candidates = view
                    .batches()
                    .map(|batch| (snapshots.matches_entitlement(&batch), batch));
                for batch in selected_entitlement_batches(candidates) {
                    for file in batch.files() {
                        if file.kind() != ResponseKind::Calendar {
                            continue;
                        }
                        references.push(SourceFileReference {
                            provider: scope.provider().to_owned(),
                            market: scope.market().to_owned(),
                            batch_id: file.batch().batch_id(),
                            file_name: file.file_name().to_owned(),
                            content_hash: file.content_hash().clone(),
                            retrieved_at: file.batch().retrieved_at(),
                        });
                    }
                }
            }
            let mut originals = Vec::new();
            let resolution = (|| {
                for reference in &references {
                    originals.push(resolve_original_checked(&snapshots, reference).map_err(
                        |e| First::new(Reason::CalendarUnproven, Source::Lineage(Box::new(e))),
                    )?);
                }
                Ok::<(), First>(())
            })();
            let result = match resolution {
                Err(first) => Err(first),
                Ok(()) => match project_calendar_candidates_checked(
                    &originals,
                    context.owner,
                    &context.policy_hash,
                ) {
                    Err(e) => Err(First::new(
                        Reason::CalendarUnproven,
                        Source::Calendar(Box::new(e)),
                    )),
                    Ok(projections) => {
                        let result = match projections.view_in() {
                            Err(e) => Err(First::new(
                                Reason::CalendarUnproven,
                                Source::Calendar(Box::new(e)),
                            )),
                            Ok(view) => {
                                let result = (|| {
                                    let eligible = eligible_calendar_date(timestamp)
                                        .map_err(|_| Reason::CalendarUnproven)?;
                                    let candidates =
                                        view.projections().iter().flat_map(|projection| {
                                            projection.days().iter().map(|day| {
                                                (
                                                    day.session_date().as_naive_date(),
                                                    day.session_type(),
                                                )
                                            })
                                        });
                                    let calendar_rows = candidates.collect::<Vec<_>>();
                                    let selected = if let Some(action) = action_request {
                                        let signal = match action {
                                            financial::PaperActionInputRequest::LatestSignal => {
                                                let floor = eligible
                                                    .as_naive_date()
                                                    .checked_sub_days(chrono::Days::new(
                                                        MAX_CALENDAR_DAYS,
                                                    ))
                                                    .ok_or(Reason::CalendarUnproven)?;
                                                calendar_rows
                                                    .iter()
                                                    .filter_map(|(date, kind)| {
                                                        (*date >= floor
                                                            && *date <= eligible.as_naive_date()
                                                            && *kind == DailySessionTypeV1::Trading)
                                                            .then_some(*date)
                                                    })
                                                    .max()
                                                    .ok_or(Reason::CalendarUnproven)?
                                            }
                                            financial::PaperActionInputRequest::Preview {
                                                signal,
                                                ..
                                            }
                                            | financial::PaperActionInputRequest::Settle {
                                                signal,
                                                ..
                                            }
                                            | financial::PaperActionInputRequest::Value {
                                                signal,
                                                ..
                                            } => signal.as_naive_date(),
                                        };
                                        if signal > eligible.as_naive_date()
                                            || !calendar_rows.iter().any(|(date, kind)| {
                                                *date == signal
                                                    && *kind == DailySessionTypeV1::Trading
                                            })
                                        {
                                            return Err(Reason::CalendarUnproven.into());
                                        }
                                        vec![signal]
                                    } else {
                                        let selected_candidates = latest_trading_candidates(
                                            calendar_rows.iter().copied(),
                                            eligible.as_naive_date(),
                                        )?;
                                        let first =
                                            TradingDate::parse(&selected_candidates[0].to_string())
                                                .map_err(|_| Reason::CalendarUnproven)?;
                                        let refs = view.projections().iter().collect::<Vec<_>>();
                                        let rows =
                                            prove_verified_calendar_suffix(&refs, first, eligible)?;
                                        let selected = latest_three_trading_sessions(
                                            &rows,
                                            eligible.as_naive_date(),
                                        )?;
                                        if selected != selected_candidates {
                                            return Err(Reason::CalendarUnproven.into());
                                        }
                                        selected.to_vec()
                                    };
                                    if let Some((start, end)) = capture_range {
                                        let proof = view
                                            .prove_range(start, end)
                                            .map_err(|_| Reason::CalendarUnproven)?;
                                        validate_paper_calendar_range(&proof, start, end)?;
                                        retain_calendar_range_proof(observations, proof)?;
                                    }
                                    Ok(selected)
                                })();
                                for local in view.into_diagnostics() {
                                    observations.local(local);
                                }
                                result
                            }
                        };
                        observations
                            .complete(Completion::Calendar(Box::new(projections.finish_local())));
                        result
                    }
                },
            };
            for original in originals {
                observations.lineage.push(original.retain_diagnostics());
            }
            observations.raw_references.extend(references);
            result
        })();
        observations.local(snapshots.into_diagnostics());
        let selected = result?;
        observations.calendars.push(selected.clone());
        observations.require_clean()?;
        Ok(selected)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use market_data::checked_close::{Close, Errno, Role};
        use std::{
            cell::{Cell, RefCell},
            pin::Pin,
            rc::Rc,
            task::{Context, Poll, Waker},
        };
        struct Counted {
            name: &'static str,
            log: Rc<RefCell<Vec<&'static str>>>,
            result: Result<(), Errno>,
        }
        impl Close for Counted {
            fn close(self) -> Result<(), Errno> {
                self.log.borrow_mut().push(self.name);
                self.result
            }
        }
        fn owner(
            attempt: &Attempt,
            name: &'static str,
            log: &Rc<RefCell<Vec<&'static str>>>,
        ) -> market_data::checked_close::Owner<Counted> {
            attempt.adopt(
                Counted {
                    name,
                    log: Rc::clone(log),
                    result: Ok(()),
                },
                Role::Source,
            )
        }
        struct Body(Box<str>);

        struct CheapBoundary<'a> {
            canceled: &'a std::sync::atomic::AtomicBool,
            started: &'a std::sync::atomic::AtomicBool,
            deadline: Instant,
            attempt: &'a Attempt,
        }

        impl financial::ActionContextGuard for CheapBoundary<'_> {
            fn revalidate(&mut self) -> Result<(), financial::PaperInputActionError> {
                panic!("final synchronous boundary must not reopen full IO validation")
            }

            fn checkpoint(&mut self) -> Result<(), financial::PaperInputActionError> {
                crate::owner_beta::paper::action::checkpoint_action_boundary(
                    self.canceled,
                    self.started,
                    self.deadline,
                    self.attempt,
                )
                .map_err(|_| financial::PaperInputActionError::Runtime)
            }
        }

        #[tokio::test]
        async fn final_commit_boundary_refuses_cancel_or_expiry_after_awaited_seal() {
            use std::sync::atomic::{AtomicBool, Ordering};
            for cancel in [true, false] {
                let canceled = AtomicBool::new(false);
                let started = AtomicBool::new(true);
                let attempt = Attempt::default();
                let deadline = Instant::now() + Duration::from_millis(100);
                let mut outer = CheapBoundary {
                    canceled: &canceled,
                    started: &started,
                    deadline,
                    attempt: &attempt,
                };
                assert_eq!(
                    financial::ActionContextGuard::checkpoint(&mut outer),
                    Ok(())
                );
                // No SQL mirror: this models the final awaited seal returning
                // after the real borrowed cancellation/deadline state changes.
                if cancel {
                    tokio::task::yield_now().await;
                    canceled.store(true, Ordering::Release);
                } else {
                    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                }
                let mut phase = ActionTransactionPhase::Active;
                let mut transaction = Some(Body("owned uncommitted transaction".into()));
                let address = transaction.as_ref().unwrap().0.as_ptr();
                let mut failures = ActionFailures::default();
                let date_called = Cell::new(false);
                let mutation = Cell::new(false);
                let result = take_after_commit_boundary(
                    &mut outer,
                    || {
                        date_called.set(true);
                        Ok(())
                    },
                    &mut phase,
                    &mut transaction,
                    &mut failures,
                );
                if result.is_ok() {
                    mutation.set(true);
                }
                assert!(matches!(
                    result,
                    Err(financial::PaperInputActionError::Runtime)
                ));
                assert!(phase == ActionTransactionPhase::Active);
                assert_eq!(transaction.as_ref().unwrap().0.as_ptr(), address);
                assert!(!mutation.get() && !date_called.get());
                assert!(
                    failures.dirty
                        && failures.first == Some(financial::PaperInputActionError::Runtime)
                );
                assert_eq!(
                    attempt.report().first(),
                    Some(market_data::checked_close::Issue::Gap(Gap::Abandoned))
                );
            }
        }

        #[tokio::test]
        async fn final_commit_boundary_valid_case_takes_transaction_only_after_fresh_date() {
            use std::sync::atomic::AtomicBool;
            let canceled = AtomicBool::new(false);
            let started = AtomicBool::new(true);
            let attempt = Attempt::default();
            let mut outer = CheapBoundary {
                canceled: &canceled,
                started: &started,
                deadline: Instant::now() + Duration::from_secs(60),
                attempt: &attempt,
            };
            let mut clock = EvaluationClock::start().unwrap();
            let date = clock
                .started_at
                .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
                .date_naive();
            tokio::task::yield_now().await;
            let mut phase = ActionTransactionPhase::Active;
            let mut transaction = Some(Body("whole commit value".into()));
            let address = transaction.as_ref().unwrap().0.as_ptr();
            let mut failures = ActionFailures::default();
            let checked = Cell::new(false);
            let value = take_after_commit_boundary(
                &mut outer,
                || {
                    checked.set(true);
                    checkpoint_action_date(&mut clock, date)
                },
                &mut phase,
                &mut transaction,
                &mut failures,
            )
            .expect("valid final boundary");
            assert!(
                checked.get()
                    && phase == ActionTransactionPhase::CommitStarted
                    && transaction.is_none()
            );
            assert_eq!(value.0.as_ptr(), address);
            assert!(!failures.dirty && failures.first.is_none() && attempt.check());
        }

        #[tokio::test]
        async fn final_commit_boundary_checks_real_date_after_seal_before_taking_slot() {
            use std::sync::atomic::AtomicBool;
            let canceled = AtomicBool::new(false);
            let started = AtomicBool::new(true);
            let attempt = Attempt::default();
            let mut outer = CheapBoundary {
                canceled: &canceled,
                started: &started,
                deadline: Instant::now() + Duration::from_secs(60),
                attempt: &attempt,
            };
            let mut clock = EvaluationClock::start().unwrap();
            let previous_date = clock
                .started_at
                .with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap())
                .date_naive()
                .pred_opt()
                .unwrap();
            tokio::task::yield_now().await;
            let mut phase = ActionTransactionPhase::Active;
            let mut transaction = Some(Body("date-refused transaction".into()));
            let mut failures = ActionFailures::default();
            let result = take_after_commit_boundary(
                &mut outer,
                || checkpoint_action_date(&mut clock, previous_date),
                &mut phase,
                &mut transaction,
                &mut failures,
            );
            assert!(matches!(
                result,
                Err(financial::PaperInputActionError::Input(
                    Reason::EligibilityExpired
                ))
            ));
            assert!(phase == ActionTransactionPhase::Active && transaction.is_some());
            assert!(
                failures.dirty
                    && failures.first
                        == Some(financial::PaperInputActionError::Input(
                            Reason::EligibilityExpired
                        ))
            );
        }

        #[test]
        fn final_commit_boundary_preserves_first_and_secondary_refusal() {
            use std::sync::atomic::AtomicBool;
            let canceled = AtomicBool::new(true);
            let started = AtomicBool::new(true);
            let attempt = Attempt::default();
            let mut outer = CheapBoundary {
                canceled: &canceled,
                started: &started,
                deadline: Instant::now() + Duration::from_secs(60),
                attempt: &attempt,
            };
            let mut failures = ActionFailures::default();
            failures.fail(financial::PaperInputActionError::Sql);
            let mut phase = ActionTransactionPhase::Active;
            let mut transaction = Some(Body("retained on secondary failure".into()));
            let result = take_after_commit_boundary(
                &mut outer,
                || panic!("cancellation must stop before date check"),
                &mut phase,
                &mut transaction,
                &mut failures,
            );
            assert!(matches!(result, Err(financial::PaperInputActionError::Sql)));
            assert_eq!(
                failures.secondary,
                vec![financial::PaperInputActionError::Runtime]
            );
            assert!(phase == ActionTransactionPhase::Active && transaction.is_some());
        }
        fn ready<T>(future: impl Future<Output = T>) -> T {
            let mut future = Box::pin(future);
            let mut context = Context::from_waker(Waker::noop());
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("ordinary immediate model unexpectedly pending"),
            }
        }

        #[derive(Debug)]
        struct LowerErrorSentinel {
            identity: Box<str>,
            drops: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
        }
        impl std::fmt::Display for LowerErrorSentinel {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("test lower error")
            }
        }
        impl std::error::Error for LowerErrorSentinel {}
        impl Drop for LowerErrorSentinel {
            fn drop(&mut self) {
                self.drops
                    .lock()
                    .unwrap()
                    .push(self.identity.as_ptr() as usize);
            }
        }

        fn lower_error(
            name: &str,
            drops: &std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
        ) -> (sqlx::Error, usize) {
            let identity = Box::<str>::from(name);
            let address = identity.as_ptr() as usize;
            (
                sqlx::Error::Io(std::io::Error::other(LowerErrorSentinel {
                    identity,
                    drops: std::sync::Arc::clone(drops),
                })),
                address,
            )
        }

        fn lower_error_address(error: &sqlx::Error) -> usize {
            let sqlx::Error::Io(error) = error else {
                panic!("lost actual SQL error")
            };
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<LowerErrorSentinel>()
                .expect("whole original typed error")
                .identity
                .as_ptr() as usize
        }

        #[test]
        fn checked_action_publication_retains_driver_errors_and_actual_cleanup_completions() {
            // Ordinary lower errors and actual custody owners, never fabricated
            // checked financial inputs. The failure state and publication below
            // are the same implementations used by ScopedActionDriver.
            for earlier_error in [true, false] {
                let drops = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let mut observations = Observations::new();
                let mut failures = ActionFailures::default();
                let (rollback_error, rollback_address) = lower_error("rollback", &drops);
                observations.rollback.push(Err(rollback_error));
                let mut first_address = None;
                let mut secondary_address = None;
                let original = if earlier_error {
                    let (first, address) = lower_error("first SQL result", &drops);
                    first_address = Some(address);
                    let (secondary, address) = lower_error("later driver postcheck", &drops);
                    secondary_address = Some(address);
                    failures.fail_first(First::sql(secondary), &mut observations);
                    Err(First::sql(first))
                } else {
                    Ok(None)
                };

                let attempt = Attempt::default();
                let captured = attempt.clone();
                let log = Rc::new(RefCell::new(Vec::new()));
                let first_owner = owner(&attempt, "chain", &log);
                let last_owner = attempt.adopt(
                    Counted {
                        name: "source-close-error",
                        log: Rc::clone(&log),
                        result: Err(Errno::Io),
                    },
                    Role::Source,
                );
                finish_session_resources(
                    Some(first_owner),
                    Some(last_owner),
                    |owner| {
                        owner.finish();
                    },
                    |owner| {
                        owner.finish();
                    },
                );
                assert_eq!(*log.borrow(), vec!["chain", "source-close-error"]);
                let invocation = RawInvocation::new(&attempt);
                let (expected_scope, captured_scope) = scopes(&invocation, &attempt, &captured);
                observations.complete(Completion::Unit(Box::new(expected_scope)));
                observations.complete(Completion::Unit(Box::new(captured_scope)));
                observations.complete(Completion::Unit(Box::new(attempt.clone().finish(Ok::<
                    (),
                    Infallible,
                >(
                    ()
                )))));
                assert_eq!(observations.completions.len(), 3);
                assert!(!observations.clean);
                observations.action_first = failures.first;
                observations.action_secondary = failures.secondary;
                let final_clock = Err(Reason::EligibilityExpired);
                let publication = Outcome {
                    original,
                    observations,
                    final_clock,
                }
                .publish_action();
                assert!(
                    drops.lock().unwrap().is_empty(),
                    "publication dropped whole lower errors before final mapping"
                );
                let outcome = financial::CheckedPaperActionOutcome::new(
                    Some(Err::<(), _>("original consumer error")),
                    publication.first,
                    publication.secondary,
                    publication.committed,
                    publication.commit_started,
                    publication.retained,
                );
                let failure = match outcome.into_result() {
                    Err(failure) => failure,
                    Ok(()) => panic!("unclean actual completion cannot publish success"),
                };
                assert!(matches!(
                    failure.original(),
                    Some(Err("original consumer error"))
                ));
                assert!(drops.lock().unwrap().is_empty());
                let lower = failure
                    .retained
                    .iter()
                    .find_map(|value| match value {
                        financial::RetainedActionValue::LowerEvidence { _outcome } => {
                            Some(_outcome.as_ref())
                        }
                        _ => None,
                    })
                    .expect("public failure owns the complete native outcome");
                if earlier_error {
                    let Err(first) = &lower.original else {
                        panic!("first failure lost")
                    };
                    let Source::Sql(error) = &first.source else {
                        panic!("first source lost")
                    };
                    assert_eq!(Some(lower_error_address(error)), first_address);
                    let Source::Sql(error) = &lower.observations.secondary[0].source else {
                        panic!("driver failure lost")
                    };
                    assert_eq!(Some(lower_error_address(error)), secondary_address);
                } else {
                    assert!(matches!(lower.original, Ok(None)));
                    assert!(lower.observations.secondary.is_empty());
                    assert_eq!(
                        failure.first_error(),
                        Some(financial::PaperInputActionError::Cleanup)
                    );
                }
                assert_eq!(
                    lower_error_address(lower.observations.rollback[0].as_ref().unwrap_err()),
                    rollback_address
                );
                assert_eq!(lower.observations.completions.len(), 3);
                for completion in &lower.observations.completions {
                    let Completion::Unit(result) = completion else {
                        panic!("completion replaced")
                    };
                    let report = result.as_ref().as_ref().unwrap_err().report();
                    assert_eq!(report.attempted(), 2);
                    assert_eq!(report.closed(), 1);
                    assert_eq!(report.unknown(), 1);
                    assert_eq!(report.live(), 0);
                }
                assert_eq!(lower.final_clock, Err(Reason::EligibilityExpired));
                drop(failure);
                let mut actual = drops.lock().unwrap().clone();
                actual.sort_unstable();
                let mut expected = vec![rollback_address];
                expected.extend(first_address);
                expected.extend(secondary_address);
                expected.sort_unstable();
                assert_eq!(
                    actual, expected,
                    "each whole payload drops exactly once at final mapping/drop"
                );
            }
        }

        #[test]
        fn checker_second_session_sql_error_retains_source_and_first_session_before_cleanup() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let mut completed = vec![(
                Some(owner(&attempt, "chain1", &log)),
                Some(owner(&attempt, "source1", &log)),
            )];
            let mut pending = None;
            assert!(
                retain_acquired(
                    Ok::<_, Body>(owner(&attempt, "source2", &log)),
                    &mut pending
                )
                .is_ok()
            );
            let first = Body("sql2".into());
            let address = first.0.as_ptr();
            let original = Err::<(), _>(first);
            assert!(pending.is_some());
            assert_eq!(completed.len(), 1);
            assert!(log.borrow().is_empty());
            completed.push((None, pending));
            drain_sessions(&mut completed, |(chain, source)| {
                finish_session_resources(
                    chain,
                    source,
                    |owner| {
                        owner.finish();
                    },
                    |owner| {
                        owner.finish();
                    },
                )
            });
            // Drain in stored session order, including the partial second
            // session; a refusal does not reverse the original cleanup policy.
            assert_eq!(*log.borrow(), vec!["chain1", "source1", "source2"]);
            match original {
                Err(body) => assert_eq!(body.0.as_ptr(), address),
                Ok(()) => panic!("lost earlier error"),
            }
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_ok());
        }
        #[test]
        fn checker_borrowed_pool_is_never_closed_and_owned_rollback_is_awaited() {
            struct Rollback {
                polls: Rc<Cell<usize>>,
                closed: Rc<Cell<bool>>,
            }
            impl Future for Rollback {
                type Output = Result<(), ()>;
                fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
                    let n = self.polls.get();
                    self.polls.set(n + 1);
                    if n == 0 {
                        Poll::Pending
                    } else {
                        self.closed.set(true);
                        Poll::Ready(Ok(()))
                    }
                }
            }
            let borrowed_pool = Cell::new(0);
            let polls = Rc::new(Cell::new(0));
            let closed = Rc::new(Cell::new(false));
            let mut future = Box::pin(finish_checker_transaction(
                Some(Body("owned transaction".into())),
                |transaction| {
                    assert_eq!(transaction.0.as_ref(), "owned transaction");
                    Rollback {
                        polls: Rc::clone(&polls),
                        closed: Rc::clone(&closed),
                    }
                },
            ));
            let mut context = Context::from_waker(Waker::noop());
            assert!(future.as_mut().poll(&mut context).is_pending());
            assert!(!closed.get());
            assert!(matches!(
                future.as_mut().poll(&mut context),
                Poll::Ready(Some(Ok(())))
            ));
            assert!(closed.get());
            assert_eq!(polls.get(), 2);
            assert_eq!(borrowed_pool.get(), 0);
        }
        #[test]
        fn checker_commit_error_consumes_slot_without_rollback_retry() {
            let mut transaction = Some(Body("sole tx".into()));
            let taken = take_for_commit(&mut transaction);
            let commit: Result<(), Body> = Err(taken);
            assert!(transaction.is_none());
            let rollback = Cell::new(0);
            let later = ready(finish_checker_transaction(transaction, |_: Body| {
                rollback.set(rollback.get() + 1);
                std::future::ready(Ok::<(), ()>(()))
            }));
            assert!(later.is_none());
            assert_eq!(rollback.get(), 0);
            assert!(commit.is_err());
        }
        #[test]
        fn checker_late_cleanup_keeps_original_eligible_report_but_blocks_publication() {
            let original = Ok::<_, Body>(Body("eligible facts".into()));
            let address = original.as_ref().ok().unwrap().0.as_ptr();
            let attempt = Attempt::default();
            let inv = RawInvocation::new(&attempt);
            attempt.gap(Gap::DatabaseClose);
            let late = inv.settle_local_in(&attempt, Ok::<(), Infallible>(()));
            assert!(late.is_err());
            assert_eq!(original.as_ref().ok().unwrap().0.as_ptr(), address);
            let mut observations = Observations::new();
            observations.complete(Completion::Unit(Box::new(late)));
            let stopped = match observations.require_clean() {
                Err(first) => first,
                Ok(()) => panic!("late refusal accepted"),
            };
            assert!(matches!(stopped.source, Source::Late));
            assert!(stopped.source_report(&observations).is_none());
            let report = publication(original, observations.clean, Ok(()), |_| {
                Reason::EvidenceMissing
            });
            assert!(matches!(report, Err(Reason::ReaderIncompatible)));
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
        #[test]
        fn checker_first_denial_and_absent_report_survive_later_failure() {
            for some in [true, false] {
                let attempt = Attempt::default();
                let inv = RawInvocation::new(&attempt);
                let original = inv.settle_local_in(
                    &attempt,
                    if some {
                        Err::<(), _>(Body("first".into()))
                    } else {
                        attempt.gap(Gap::Unbound);
                        Ok::<(), Body>(())
                    },
                );
                let address = original
                    .as_ref()
                    .err()
                    .and_then(Failure::body)
                    .map(|body| body.0.as_ptr());
                let later = inv.settle_local_in(&attempt, Ok::<(), Body>(()));
                let mut retained = Vec::new();
                let mut slot = None;
                let result = retain_replay(original, None, later, &mut slot, &mut retained);
                match result {
                    Err(error) => {
                        assert_eq!(error.body().map(|body| body.0.as_ptr()), address);
                        assert_eq!(error.body().is_some(), some)
                    }
                    Ok(()) => panic!("lost real Some/None failure"),
                }
                assert_eq!(retained.len(), 1);
                assert!(slot.is_none());
                let _ = attempt.finish(Ok::<(), Infallible>(()));
            }
            // A plain policy denial is kept first, including its absence of a source report.
            let observations = Observations::new();
            let policy = First::from(Reason::CalendarUnproven);
            assert!(policy.source_report(&observations).is_none());
            let first: Result<(), First> = Err(policy);
            assert!(matches!(
                publication(first, false, Err(Reason::EligibilityExpired), |e| e.reason),
                Err(Reason::CalendarUnproven)
            ));
        }
        #[test]
        fn checker_all_foreign_scopes_are_visited_when_dirty() {
            let actual = Attempt::default();
            let expected = Attempt::default();
            let captured = Attempt::default();
            actual.gap(Gap::Abandoned);
            let inv = RawInvocation::new(&actual);
            let (a, b) = scopes(&inv, &expected, &captured);
            assert!(a.is_err());
            assert!(b.is_err());
            assert!(!actual.check());
            assert!(!expected.check());
            assert!(!captured.check());
            assert!(actual.finish(Ok::<(), Infallible>(())).is_err());
            assert!(expected.finish(Ok::<(), Infallible>(())).is_err());
            assert!(captured.finish(Ok::<(), Infallible>(())).is_err());
        }
        #[test]
        fn checker_both_raw_refreshes_finish_calendar_then_kis_before_next_phase() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            for (calendar, kis, next) in [
                ("calendar1", "kis1", "db2"),
                ("calendar2", "kis2", "final_calendar"),
            ] {
                let cal = owner(&attempt, calendar, &log);
                let kis = owner(&attempt, kis, &log);
                let original = Ok::<_, Body>(Body("actual returned match".into()));
                finish_pair(Some(cal), Some(kis), |owner| {
                    owner.finish();
                });
                assert_eq!(
                    original.as_ref().ok().unwrap().0.as_ref(),
                    "actual returned match"
                );
                log.borrow_mut().push(next);
            }
            assert_eq!(
                *log.borrow(),
                vec![
                    "calendar1",
                    "kis1",
                    "db2",
                    "calendar2",
                    "kis2",
                    "final_calendar"
                ]
            );
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_ok());
        }
        #[test]
        fn checker_three_sessions_and_both_database_readbacks_keep_exact_order() {
            let dates = [1, 2, 3];
            let mut trace = Vec::new();
            for date in session_order(&dates) {
                trace.push(("initial", *date));
            }
            for date in session_order(&dates) {
                trace.push(("readback", *date));
            }
            assert_eq!(
                trace,
                vec![
                    ("initial", 1),
                    ("initial", 2),
                    ("initial", 3),
                    ("readback", 1),
                    ("readback", 2),
                    ("readback", 3)
                ]
            );
            // Only date traversal is modelled; real SQL/commit/revalidation chronology is a separate static obligation.
        }
        #[test]
        fn checker_expiry_during_final_cleanup_prevents_publication() {
            let at = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).single().unwrap();
            let mut clock = EvaluationClock::synthetic(at).unwrap();
            let original = Ok::<_, Body>(Body("eligible before cleanup".into()));
            let elapsed = Duration::from_secs(u64::try_from(ELIGIBILITY_TTL_SECONDS).unwrap());
            let after = clock
                .check_sample(
                    at + chrono::Duration::seconds(ELIGIBILITY_TTL_SECONDS),
                    elapsed,
                )
                .map(|_| ());
            assert!(matches!(
                publication(original, true, after, |_| Reason::EvidenceMissing),
                Err(Reason::EligibilityExpired)
            ));
        }
        #[test]
        fn checked_action_panic_uses_existing_unwind_and_records_abandoned() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _unwind = Unwind {
                    attempt: &attempt,
                    armed: true,
                };
                let _owner = owner(&attempt, "real-owner-drop", &log);
                panic!("entered local checked callback");
            }));
            assert!(result.is_err());
            assert_eq!(*log.borrow(), vec!["real-owner-drop"]);
            let report = attempt.report();
            assert_eq!(report.attempted(), 1);
            assert_eq!(report.closed(), 1);
            assert_eq!(report.live(), 0);
            assert_eq!(
                report.first(),
                Some(market_data::checked_close::Issue::Gap(Gap::Abandoned))
            );
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }

        #[test]
        fn checker_unwind_gaps_and_async_cancellation_makes_no_completion_claim() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let outside = owner(&attempt, "outer", &log);
            {
                let _armed = Unwind {
                    attempt: &attempt,
                    armed: true,
                };
            }
            assert!(!attempt.check());
            assert_eq!(attempt.report().attempted(), 0);
            assert_eq!(attempt.report().live(), 1);
            let rollback = Cell::new(0);
            let future = finish_checker_transaction(Some(Body("unpolled tx".into())), |_: Body| {
                rollback.set(1);
                std::future::ready(Ok::<(), ()>(()))
            });
            drop(future);
            assert_eq!(rollback.get(), 0);
            outside.finish();
            assert_eq!(*log.borrow(), vec!["outer"]);
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
        #[test]
        fn actual_unbound_raw_or_daily_public_refusal_has_zero_next_business_effects() {
            let attempt = Attempt::default();
            let inv = RawInvocation::new(&attempt);
            let effects = Cell::new(0);
            let scope = ApprovedRawScope::new("kis", "kr", 10001).unwrap();
            let values = ScanLimitValues {
                max_manifest_bytes: 1024,
                max_manifest_records: 1,
                max_batches: 1,
                max_files_per_batch: 1,
                max_files: 1,
                max_batch_metadata_bytes: 1024,
                max_source_file_bytes: 1024,
                max_retained_bytes: 4096,
                max_open_descriptors: 32,
                max_data_root_bytes: 4096,
                max_data_root_components: 16,
                max_metadata_json_depth: 16,
            };
            let result = CommittedRawReader::new(Path::new("/not-opened-unbound-checker"))
                .snapshot_checked_in(
                    &inv,
                    &attempt,
                    &attempt,
                    scope,
                    ScanLimits::new(values).unwrap(),
                )
                .map(|snapshot| {
                    effects.set(effects.get() + 1);
                    snapshot.finish_local()
                });
            match result {
                Err(failure) => assert!(matches!(
                    failure.kind(),
                    market_data::daily_price_only::source::SnapshotFirstKind::Binding
                )),
                Ok(_) => panic!("None gate unexpectedly opened"),
            }
            assert_eq!(effects.get(), 0);
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
        #[test]
        fn checker_raw_aggregate_retained_before_failed_postcheck() {
            struct Aggregate {
                identities: Box<str>,
                references: Box<[u8]>,
                drops: Rc<Cell<usize>>,
            }
            impl Drop for Aggregate {
                fn drop(&mut self) {
                    self.drops.set(self.drops.get() + 1);
                }
            }
            let drops = Rc::new(Cell::new(0));
            let mut retained = Vec::with_capacity(2);
            retained.push(Aggregate {
                identities: "earlier".into(),
                references: vec![1].into_boxed_slice(),
                drops: Rc::clone(&drops),
            });
            let retained_slot = retained.as_ptr().wrapping_add(retained.len());
            let returned = Aggregate {
                identities: "actual returned identities".into(),
                references: vec![2, 3].into_boxed_slice(),
                drops: Rc::clone(&drops),
            };
            let identities = returned.identities.as_ptr();
            let references = returned.references.as_ptr();
            let first = Body("actual postcheck failure".into());
            let first_identity = first.0.as_ptr();
            let result = retain_before_postcheck(&mut retained, returned, |actual| {
                assert_eq!(std::ptr::from_ref(actual), retained_slot);
                assert_eq!(actual.identities.as_ptr(), identities);
                assert_eq!(actual.references.as_ptr(), references);
                assert_eq!(drops.get(), 0);
                Err(first)
            });
            let first = match result {
                Err(first) => first,
                Ok(()) => panic!("actual postcheck failure was lost"),
            };
            assert_eq!(first.0.as_ptr(), first_identity);
            assert_eq!(retained.len(), 2);
            assert_eq!(&*retained[0].identities, "earlier");
            assert_eq!(retained[1].identities.as_ptr(), identities);
            assert_eq!(retained[1].references.as_ptr(), references);
            assert_eq!(drops.get(), 0);
            let mut observations = Vec::new();
            observations.extend(retained);
            assert_eq!(observations.len(), 2);
            assert_eq!(observations[1].identities.as_ptr(), identities);
            assert_eq!(observations[1].references.as_ptr(), references);
            assert_eq!(drops.get(), 0);
            drop(observations);
            assert_eq!(drops.get(), 2);
        }
    }
}
