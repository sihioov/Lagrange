//! Explicit, ignored functional integration entry points. The outer fixture is
//! SYNTHETIC TEST AUTHORITY. The public checker still opens every genuine
//! protected holder and runs the original SQL, Raw and Daily validators.
//! No fixture value constructs an approved artifact or execution capability.

use super::*;
use crate::owner_beta::paper::calculation::{
    BetaAccountStateIdentity, calculate_beta_preview, execute_beta_session_open,
    validate_next_trading_session, value_beta_close, verify_t_e_prefix,
};
use domain::{ContentHash, Currency, InstrumentId, Money, TradingDate};
use portfolio_model::{CostProfile, LedgerState};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use std::collections::BTreeSet;
use std::{
    cell::Cell,
    fs::{Metadata, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

const FIXTURE_ROOT: &str = "/run/r5-paper-checker-integration";
const FIXTURE_LIMIT: u64 = 65_536;
type CheckerIdentity = (
    String,
    String,
    String,
    i32,
    bool,
    bool,
    bool,
    bool,
    bool,
    i64,
    Option<String>,
    Option<String>,
);
type PhaseIdentity = (
    String,
    String,
    String,
    bool,
    bool,
    bool,
    bool,
    bool,
    i64,
    Option<String>,
);

thread_local! {
    // Current-thread Tokio execution is required. This affects only this
    // private test clock; protected Current's native clocks are never changed.
    static TEST_WALL: Cell<Option<DateTime<Utc>>> = const { Cell::new(None) };
}

pub(super) fn start_override() -> Option<DateTime<Utc>> {
    TEST_WALL.get()
}

pub(super) fn sample_override(
    started_at: DateTime<Utc>,
    elapsed: Duration,
) -> Option<DateTime<Utc>> {
    TEST_WALL.get().filter(|at| *at == started_at)?;
    started_at.checked_add_signed(chrono::Duration::from_std(elapsed).ok()?)
}

struct FixtureClock;
impl FixtureClock {
    fn enter(at: DateTime<Utc>) -> Result<Self, ()> {
        TEST_WALL.with(|slot| {
            if slot.get().is_some() {
                return Err(());
            }
            slot.set(Some(at));
            Ok(Self)
        })
    }
}
impl Drop for FixtureClock {
    fn drop(&mut self) {
        TEST_WALL.set(None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Case {
    Normal,
    Manual,
    Tamper,
    WrongInput,
    FailedE,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Manual => "manual",
            Self::Tamper => "tamper",
            Self::WrongInput => "wrong_input",
            Self::FailedE => "failed_e",
        }
    }
    fn reason(self) -> Option<Reason> {
        match self {
            Self::Normal => None,
            // These variants corrupt a Source chain before it is admitted:
            // manual origin and changed hash-linked event bytes are rejected by
            // the real ProtectedSourceChain open, mapped to OriginUnproven.
            Self::Manual | Self::Tamper => Some(Reason::OriginUnproven),
            // A well-formed selector for another Daily config disagrees with
            // the freshly reopened complete protected Current binding.
            Self::WrongInput => Some(Reason::CohortChanged),
            // The failed latest-session recommendation keeps report-only
            // latest-three eligibility closed even when its price was admitted.
            Self::FailedE => Some(Reason::OriginUnproven),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: String,
    classification: String,
    run_nonce: String,
    case: Case,
    port: u16,
    database: String,
    worker_password: String,
    server_version_num: i32,
    evaluated_at: DateTime<Utc>,
    sessions: [NaiveDate; 3],
    binding: Value,
}

fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_fixture(fixture: &Fixture, nonce: &str, case: Case) -> Result<(), ()> {
    if fixture.schema != "r5-checker-functional-fixture-v1"
        || fixture.classification != "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN"
        || !lower_hex(nonce, 32)
        || fixture.run_nonce != nonce
        || fixture.case != case
        || fixture.port == 0
        || fixture.database != format!("r5_checker_{nonce}")
        || !lower_hex(&fixture.worker_password, 64)
        || fixture.server_version_num != 180004
        || fixture.sessions.windows(2).any(|pair| pair[0] >= pair[1])
        || fixture.sessions[2]
            > eligible_calendar_date(UtcTimestamp::from_datetime(fixture.evaluated_at))
                .map_err(|_| ())?
                .as_naive_date()
    {
        return Err(());
    }
    Ok(())
}

fn private_file(meta: &Metadata, uid: u32) -> bool {
    meta.is_file() && meta.uid() == uid && meta.mode() & 0o7777 == 0o600 && meta.nlink() == 1
}

fn identity(meta: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    )
}

fn private_parent(path: &Path, uid: u32) -> Result<(), ()> {
    let parent = path.parent().ok_or(())?;
    for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let meta = std::fs::symlink_metadata(ancestor).map_err(|_| ())?;
        if !meta.is_dir()
            || (ancestor == parent && (meta.uid() != uid || meta.mode() & 0o7777 != 0o700))
        {
            return Err(());
        }
    }
    Ok(())
}

fn read_fixture_file() -> Result<(String, Vec<u8>), ()> {
    // No ambient DSN, role override or libpq alternate endpoint is accepted.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| {
            key.starts_with("PG")
                || key.starts_with("DB_")
                || matches!(key, "DATABASE_URL" | "DATABASE_URL_FILE")
        }) {
            return Err(());
        }
    }
    let nonce = std::env::var("R5_INTEG_NONCE").map_err(|_| ())?;
    let pin = std::env::var("R5_INTEG_FIXTURE_SHA256").map_err(|_| ())?;
    if !lower_hex(&nonce, 32) || !lower_hex(&pin, 64) {
        return Err(());
    }
    let path = PathBuf::from(format!("{FIXTURE_ROOT}/{nonce}/fixture.json"));
    // SAFETY: argument-free query of the actual fixture reader's identity.
    let uid = unsafe { libc::geteuid() };
    private_parent(&path, uid)?;
    let named = std::fs::symlink_metadata(&path).map_err(|_| ())?;
    if !private_file(&named, uid) || named.len() > FIXTURE_LIMIT {
        return Err(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| ())?;
    let before = file.metadata().map_err(|_| ())?;
    if !private_file(&before, uid) || identity(&before) != identity(&named) {
        return Err(());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(FIXTURE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    let after = file.metadata().map_err(|_| ())?;
    let named_after = std::fs::symlink_metadata(&path).map_err(|_| ())?;
    private_parent(&path, uid)?;
    if bytes.len() as u64 > FIXTURE_LIMIT
        || !private_file(&after, uid)
        || !private_file(&named_after, uid)
        || identity(&before) != identity(&after)
        || identity(&after) != identity(&named_after)
        || format!("{:x}", Sha256::digest(&bytes)) != pin
    {
        return Err(());
    }
    Ok((nonce, bytes))
}

fn load_fixture(case: Case) -> Result<Fixture, ()> {
    let (nonce, bytes) = read_fixture_file()?;
    let fixture: Fixture = serde_json::from_slice(&bytes).map_err(|_| ())?;
    validate_fixture(&fixture, &nonce, case)?;
    Ok(fixture)
}

fn matches_expected(fixture: &Fixture, report: &PaperEligibilityReport) -> bool {
    match fixture.case.reason() {
        Some(reason) => {
            report.decision() == PaperEligibilityDecision::Ineligible
                && report.reason_codes().collect::<Vec<_>>() == [reason.code()]
        }
        None => {
            report.decision() == PaperEligibilityDecision::Eligible
                && report.reason_codes().len() == 0
                && report.sessions() == fixture.sessions.map(|date| date.to_string())
                && report.evaluated_at() == fixture.evaluated_at
                && report.expires_at() > report.evaluated_at()
        }
    }
}

async fn run_case(case: Case) {
    let fixture = load_fixture(case).unwrap_or_else(|()| panic!("R5_INTEG_FIXTURE_REJECTED"));
    let binding = wire::canonical(&fixture.binding)
        .ok()
        .and_then(|bytes| UntrustedPaperContractV1::decode(&bytes).ok())
        .unwrap_or_else(|| panic!("R5_INTEG_BINDING_REJECTED"));
    let options = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(fixture.port)
        .database(&fixture.database)
        .username("worker")
        .password(&fixture.worker_password)
        .ssl_mode(PgSslMode::Disable);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .unwrap_or_else(|_| panic!("R5_INTEG_DB_CONNECT_REJECTED"));
    // Keep the whole real SQL result until after the actual owned pool close.
    let original = async {
        let row: CheckerIdentity = sqlx::query_as(
            "SELECT current_database()::text,current_user::text,session_user::text,\
             current_setting('server_version_num')::integer,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls,\
             (SELECT count(*) FROM public.r5_checker_fixture_identity),\
             (SELECT min(nonce) FROM public.r5_checker_fixture_identity),\
             (SELECT min(case_name) FROM public.r5_checker_fixture_identity)\
             FROM pg_roles WHERE rolname=current_user")
            .fetch_one(&pool).await?;
        let identity_ok = row.0 == fixture.database && row.1 == "worker" && row.2 == "worker"
            && row.3 == fixture.server_version_num && !row.4 && !row.5 && !row.6 && !row.7 && !row.8
            && row.9 == 1 && row.10.as_deref() == Some(fixture.run_nonce.as_str())
            && row.11.as_deref() == Some(case.name());
        if !identity_ok { return Ok::<_, sqlx::Error>(None); }
        let _clock = FixtureClock::enter(fixture.evaluated_at)
            .unwrap_or_else(|()| panic!("R5_INTEG_CLOCK_ALREADY_SET"));
        // The genuine public entry point. No evaluator/model/JSON success substitute.
        Ok(Some(check_paper_eligibility(&pool, &binding).await))
    }.await;
    let checker_left_pool_open = !pool.is_closed();
    pool.close().await;
    let report = original
        .unwrap_or_else(|_| panic!("R5_INTEG_DB_IDENTITY_QUERY_FAILED"))
        .unwrap_or_else(|| panic!("R5_INTEG_DB_IDENTITY_REJECTED"));
    assert!(
        checker_left_pool_open && pool.is_closed(),
        "R5_INTEG_POOL_OWNERSHIP"
    );
    assert!(
        matches_expected(&fixture, &report),
        "R5_INTEG_VERDICT_MISMATCH case={} decision={:?} reasons={:?}",
        case.name(),
        report.decision(),
        report.reason_codes().collect::<Vec<_>>()
    );
    println!(
        "R5_INTEG actual_checker=true classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN case={} decision={:?} sessions={} pool_close_awaited=true",
        case.name(),
        report.decision(),
        report.sessions().len()
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-owned isolated PG and genuine protected fixture required; not a host test"]
async fn actual_checker_normal() {
    run_case(Case::Normal).await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-owned isolated PG and manual Source mutation required"]
async fn actual_checker_manual_refusal() {
    run_case(Case::Manual).await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-owned isolated PG and hash-linked Source byte mutation required"]
async fn actual_checker_tamper_refusal() {
    run_case(Case::Tamper).await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-owned isolated PG and other well-formed Current selector required"]
async fn actual_checker_wrong_input_refusal() {
    run_case(Case::WrongInput).await;
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-owned isolated PG with admitted E prices and a failed E recommendation required"]
async fn actual_checker_failed_e_recommendation_refusal() {
    run_case(Case::FailedE).await;
}

fn trading_date(date: NaiveDate) -> TradingDate {
    TradingDate::parse(&date.to_string()).expect("fixture trading date")
}

fn financial_request(fixture: &Fixture) -> financial::PaperCalculationInputRequest {
    let t = fixture.sessions[1];
    let e = fixture.sessions[2];
    financial::PaperCalculationInputRequest::new([t, e], Some((trading_date(t), trading_date(e))))
}

#[derive(Clone, Copy)]
enum FinancialCaptureHarnessError {
    Database,
    Identity,
    Clock,
}

enum FinancialCaptureOutcome {
    Observed(financial::ObservedPaperCalculationInputs),
    Denied(financial::PaperCalculationInputError),
}

async fn run_financial_capture_case(case: Case) {
    let fixture =
        load_fixture(case).unwrap_or_else(|()| panic!("R5_INTEG_FINANCIAL_FIXTURE_REJECTED"));
    let binding = wire::canonical(&fixture.binding)
        .ok()
        .and_then(|bytes| UntrustedPaperContractV1::decode(&bytes).ok())
        .unwrap_or_else(|| panic!("R5_INTEG_FINANCIAL_BINDING_REJECTED"));
    let options = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(fixture.port)
        .database(&fixture.database)
        .username("worker")
        .password(&fixture.worker_password)
        .ssl_mode(PgSslMode::Disable);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .unwrap_or_else(|_| panic!("R5_INTEG_FINANCIAL_DB_CONNECT_REJECTED"));

    // Keep the same exact disposable SQL identity gate as the original checker
    // selectors, and await our owned pool close on every returned outcome.
    let original: Result<FinancialCaptureOutcome, FinancialCaptureHarnessError> = async {
        let row: CheckerIdentity = sqlx::query_as(
            "SELECT current_database()::text,current_user::text,session_user::text,\
             current_setting('server_version_num')::integer,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls,\
             (SELECT count(*) FROM public.r5_checker_fixture_identity),\
             (SELECT min(nonce) FROM public.r5_checker_fixture_identity),\
             (SELECT min(case_name) FROM public.r5_checker_fixture_identity)\
             FROM pg_roles WHERE rolname=current_user",
        )
        .fetch_one(&pool)
        .await
        .map_err(|_| FinancialCaptureHarnessError::Database)?;
        let identity_ok = row.0 == fixture.database
            && row.1 == "worker"
            && row.2 == "worker"
            && row.3 == fixture.server_version_num
            && !row.4
            && !row.5
            && !row.6
            && !row.7
            && !row.8
            && row.9 == 1
            && row.10.as_deref() == Some(fixture.run_nonce.as_str())
            && row.11.as_deref() == Some(case.name());
        if !identity_ok {
            return Err(FinancialCaptureHarnessError::Identity);
        }
        let _clock = FixtureClock::enter(fixture.evaluated_at)
            .map_err(|()| FinancialCaptureHarnessError::Clock)?;
        let result = financial::read_paper_calculation_inputs(
            &pool,
            &binding,
            &financial_request(&fixture),
        )
        .await;
        Ok(match result {
            Ok(observed) => FinancialCaptureOutcome::Observed(observed),
            Err(error) => FinancialCaptureOutcome::Denied(error),
        })
    }
    .await;
    let checker_left_pool_open = !pool.is_closed();
    pool.close().await;
    assert!(
        checker_left_pool_open && pool.is_closed(),
        "R5_INTEG_FINANCIAL_POOL_OWNERSHIP"
    );

    let outcome = match original {
        Ok(outcome) => outcome,
        Err(FinancialCaptureHarnessError::Database) => {
            panic!("R5_INTEG_FINANCIAL_DB_IDENTITY_QUERY_FAILED")
        }
        Err(FinancialCaptureHarnessError::Identity) => {
            panic!("R5_INTEG_FINANCIAL_DB_IDENTITY_REJECTED")
        }
        Err(FinancialCaptureHarnessError::Clock) => {
            panic!("R5_INTEG_FINANCIAL_CLOCK_ALREADY_SET")
        }
    };

    match (case.reason(), outcome) {
        (Some(reason), FinancialCaptureOutcome::Denied(error)) => {
            assert!(
                error == financial::PaperCalculationInputError::Eligibility(reason),
                "R5_INTEG_FINANCIAL_DENIAL_MISMATCH"
            );
            println!(
                "R5_INTEG actual_financial_capture=true classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN case={} exact_denial={:?} pool_close_awaited=true",
                case.name(),
                reason
            );
        }
        (Some(_), FinancialCaptureOutcome::Observed(_)) => {
            panic!("R5_INTEG_FINANCIAL_NEGATIVE_CAPTURE_SUCCEEDED")
        }
        (None, FinancialCaptureOutcome::Denied(error)) => {
            panic!("R5_INTEG_FINANCIAL_NORMAL_CAPTURE_REJECTED error={error:?}")
        }
        (None, FinancialCaptureOutcome::Observed(observed)) => {
            verify_financial_capture_and_calculation(&fixture, &observed);
            println!(
                "R5_INTEG actual_financial_capture=true actual_pure_calculation=true classification=SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN case={} sessions={} pool_close_awaited=true ledger=synthetic_fixture_only",
                case.name(),
                observed.sessions().len()
            );
        }
    }
}

fn expected_etf11_instruments() -> BTreeSet<InstrumentId> {
    market_data::KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| {
            InstrumentId::parse(&format!("{symbol}.KRX")).expect("fixed ETF11 instrument")
        })
        .collect()
}

fn assert_complete_positive_raw_prices(
    prices: &std::collections::BTreeMap<InstrumentId, domain::Price>,
) {
    let keys = prices.keys().cloned().collect::<BTreeSet<_>>();
    assert!(
        keys == expected_etf11_instruments(),
        "R5_INTEG_FINANCIAL_RAW_ETF11_COVERAGE"
    );
    assert!(
        prices.values().all(|price| price.amount().is_positive()),
        "R5_INTEG_FINANCIAL_RAW_PRICE_INVALID"
    );
}

fn verify_financial_capture_and_calculation(
    fixture: &Fixture,
    observed: &financial::ObservedPaperCalculationInputs,
) {
    assert!(
        observed.report().decision() == PaperEligibilityDecision::Eligible
            && matches_expected(fixture, observed.report()),
        "R5_INTEG_FINANCIAL_ELIGIBILITY_OR_SESSIONS"
    );
    assert!(
        observed.current_binding() == &fixture.binding,
        "R5_INTEG_FINANCIAL_CURRENT_BINDING"
    );

    let t_date = fixture.sessions[1];
    let e_date = fixture.sessions[2];
    let t = observed
        .sessions()
        .get(&t_date)
        .unwrap_or_else(|| panic!("R5_INTEG_FINANCIAL_T_CAPTURE_MISSING"));
    let e = observed
        .sessions()
        .get(&e_date)
        .unwrap_or_else(|| panic!("R5_INTEG_FINANCIAL_E_CAPTURE_MISSING"));
    let process_observed_at = Utc::now();
    for (date, session) in [(t_date, t), (e_date, e)] {
        let input = session.input();
        let target = session.target();
        let snapshot = session.snapshot();
        let readback = session.readback_value();
        let run_id = input.run_id().to_string();
        let strategy_config_id = input.strategy_config_id().to_string();
        let date_text = date.to_string();
        let pins = input.pins();
        let snapshot_pins = snapshot.pins();
        let target_pins = target.pins();
        assert!(
            input.as_of() == trading_date(date)
                && target.as_of() == trading_date(date)
                && snapshot.as_of() == trading_date(date)
                && readback["slot"]["session_date"].as_str() == Some(date_text.as_str())
                && readback["candidate_run_id"].as_str() == Some(run_id.as_str())
                && readback["run"]["run_id"].as_str() == Some(run_id.as_str())
                && readback["run"]["job_id"].as_str() == readback["job_id"].as_str()
                && readback["run"]["attempt_id"].as_str() == readback["attempt_id"].as_str()
                && fixture.binding["cohort"]["strategy_config_id"].as_str()
                    == Some(strategy_config_id.as_str()),
            "R5_INTEG_FINANCIAL_SQL_INPUT_IDENTITY"
        );
        assert!(
            target.validate_hash().is_ok()
                && readback["factor_snapshot_sha256"].as_str()
                    == Some(target.factor_snapshot_sha256().as_str())
                && readback["target_snapshot_sha256"].as_str()
                    == Some(target.target_snapshot_sha256().as_str()),
            "R5_INTEG_FINANCIAL_RECOMPUTED_FACTOR_TARGET_HASHES"
        );
        assert!(
            target_pins == pins
                && snapshot_pins.content_sha256.as_str() == pins.content_sha256().as_str()
                && snapshot_pins.manifest_sha256.as_str() == pins.manifest_sha256().as_str()
                && snapshot_pins.admission_sha256.as_str() == pins.admission_sha256().as_str()
                && snapshot_pins.policy_sha256.as_str() == pins.policy_sha256().as_str()
                && readback["pins"]["content_sha256"].as_str()
                    == Some(pins.content_sha256().as_str())
                && readback["pins"]["manifest_sha256"].as_str()
                    == Some(pins.manifest_sha256().as_str())
                && readback["pins"]["admission_sha256"].as_str()
                    == Some(pins.admission_sha256().as_str())
                && readback["pins"]["policy_sha256"].as_str()
                    == Some(pins.policy_sha256().as_str()),
            "R5_INTEG_FINANCIAL_PRICE_PINS"
        );

        let availability = session.availability();
        let admitted_at = readback["admitted_at"]
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        assert!(
            admitted_at == Some(availability.input_admitted_at)
                && availability.source_original_retrieved_at <= process_observed_at
                && availability.input_admitted_at <= process_observed_at
                && availability.usable_at <= process_observed_at
                && availability.usable_at == session.observed_at()
                && session.observed_at() <= process_observed_at,
            "R5_INTEG_FINANCIAL_AVAILABILITY_ORDER"
        );
    }

    let proof = observed
        .calendar_range_proof()
        .unwrap_or_else(|| panic!("R5_INTEG_FINANCIAL_CALENDAR_PROOF_MISSING"));
    assert!(
        proof
            .days()
            .first()
            .is_some_and(|day| day.date() == trading_date(t_date))
            && proof
                .days()
                .last()
                .is_some_and(|day| day.date() == trading_date(e_date))
            && validate_next_trading_session(
                t.snapshot(),
                e.snapshot(),
                proof,
                proof.owner(),
                proof.policy_sha256(),
            )
            .is_ok()
            && verify_t_e_prefix(t.snapshot(), e.snapshot()).is_ok(),
        "R5_INTEG_FINANCIAL_CALENDAR_OR_PREFIX"
    );

    // This starting balance and empty account are synthetic test data only.
    // They are not read from SQL and do not authorize runtime settlement.
    let state = LedgerState::new(
        Money::parse("10000000.0000", Currency::KRW).expect("synthetic KRW balance"),
        CostProfile::resolve("KRX_ETF_DEFAULT").expect("adopted example cost profile"),
    );
    let account_id = uuid::Uuid::from_u128(0x5255_5354_4658_0000_0000_0000_0000_0001);
    let account_identity = BetaAccountStateIdentity {
        account_id,
        state_version: 0,
        state_sha256: ContentHash::from_bytes(b"synthetic fixture account state"),
    };
    let preview = calculate_beta_preview(
        t.snapshot(),
        t.input(),
        t.target(),
        account_identity,
        None,
        &state,
    )
    .unwrap_or_else(|_| panic!("R5_INTEG_FINANCIAL_T_PREVIEW_REJECTED"));
    assert!(
        preview.lineage().run_id() == t.input().run_id()
            && preview.lineage().strategy_config_id() == t.input().strategy_config_id()
            && preview.lineage().factor_snapshot_sha256() == t.target().factor_snapshot_sha256()
            && preview.lineage().target_snapshot_sha256() == t.target().target_snapshot_sha256(),
        "R5_INTEG_FINANCIAL_PREVIEW_LINEAGE"
    );
    assert_complete_positive_raw_prices(preview.raw_close_prices());

    let execution = execute_beta_session_open(
        t.snapshot(),
        e.snapshot(),
        t.input(),
        t.target(),
        account_id,
        e.snapshot().as_of(),
        proof,
        proof.owner(),
        proof.policy_sha256(),
        None,
        &state,
    )
    .unwrap_or_else(|_| panic!("R5_INTEG_FINANCIAL_E_EXECUTION_REJECTED"));
    assert_complete_positive_raw_prices(execution.raw_open_prices());
    let valuation = value_beta_close(e.snapshot(), e.snapshot().as_of(), execution.state())
        .unwrap_or_else(|_| panic!("R5_INTEG_FINANCIAL_E_VALUATION_REJECTED"));
    assert_complete_positive_raw_prices(&valuation.raw_close_prices);
    assert!(
        valuation.equity.amount().is_positive(),
        "R5_INTEG_FINANCIAL_EQUITY_INVALID"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "Root-owned disposable PG plus genuine checked normal capture fixture required"]
async fn actual_financial_capture_normal() {
    run_financial_capture_case(Case::Normal).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "Root-owned disposable PG plus genuine manual-origin refusal fixture required"]
async fn actual_financial_capture_manual_refusal() {
    run_financial_capture_case(Case::Manual).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "Root-owned disposable PG plus genuine tampered-source refusal fixture required"]
async fn actual_financial_capture_tamper_refusal() {
    run_financial_capture_case(Case::Tamper).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "Root-owned disposable PG plus genuine other-input refusal fixture required"]
async fn actual_financial_capture_wrong_input_refusal() {
    run_financial_capture_case(Case::WrongInput).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[ignore = "Root-owned isolated PG with admitted E prices and a failed E recommendation required"]
async fn actual_financial_capture_failed_e_recommendation_refusal() {
    run_financial_capture_case(Case::FailedE).await;
}

#[test]
fn financial_request_uses_the_last_two_witnessed_sessions_without_civil_day_assumptions() {
    let mut fixture = ordinary_fixture(Case::Normal);
    fixture.sessions = [
        "2026-08-27".parse().expect("first witnessed date"),
        "2026-08-28".parse().expect("T witnessed date"),
        "2026-08-31".parse().expect("E witnessed date"),
    ];
    let request = financial_request(&fixture);
    assert!(
        request.price_dates().collect::<Vec<_>>() == vec![fixture.sessions[1], fixture.sessions[2]]
            && request.calendar_range()
                == Some((
                    trading_date(fixture.sessions[1]),
                    trading_date(fixture.sessions[2])
                )),
        "R5_INTEG_FINANCIAL_REQUEST_DATES"
    );
}

#[test]
fn action_request_matrix_keeps_exact_t_and_price_only_e_selectors() {
    let t = trading_date("2026-09-02".parse().expect("T date"));
    let e = trading_date("2026-09-03".parse().expect("E date"));
    let run_id = uuid::Uuid::from_u128(0x1234);

    let latest = financial::PaperActionInputRequest::LatestSignal.calculation_request();
    assert!(latest.price_dates().next().is_none());
    assert_eq!(latest.calendar_range(), None);

    let preview = financial::PaperActionInputRequest::Preview {
        signal: t,
        effective: e,
        run_id,
    }
    .calculation_request();
    assert_eq!(
        preview.price_dates().collect::<Vec<_>>(),
        vec![t.as_naive_date()]
    );
    assert_eq!(preview.calendar_range(), Some((t, e)));
    assert!(preview.requires_recommendation(t.as_naive_date()));
    assert_eq!(preview.expected_run_id(t.as_naive_date()), Some(run_id));

    for request in [
        financial::PaperActionInputRequest::Settle {
            signal: t,
            effective: e,
            run_id,
        },
        financial::PaperActionInputRequest::Value {
            signal: t,
            effective: e,
            run_id,
        },
    ] {
        let inputs = request.calculation_request();
        assert_eq!(
            inputs.price_dates().collect::<Vec<_>>(),
            vec![t.as_naive_date(), e.as_naive_date()]
        );
        assert_eq!(inputs.calendar_range(), Some((t, e)));
        assert!(inputs.requires_recommendation(t.as_naive_date()));
        assert!(!inputs.requires_recommendation(e.as_naive_date()));
        assert!(inputs.is_price_only(e.as_naive_date()));
        assert_eq!(inputs.expected_run_id(t.as_naive_date()), Some(run_id));
        assert_eq!(inputs.expected_run_id(e.as_naive_date()), None);
    }
}

#[test]
fn checked_action_failure_retains_original_error_and_secondary_outcomes() {
    let outcome = financial::CheckedPaperActionOutcome::new(
        Some(Err::<(), _>("consumer-original")),
        Some(financial::PaperInputActionError::Sql),
        vec![financial::PaperInputActionError::Cleanup],
        false,
        false,
        vec![financial::RetainedActionValue::SqlValue(
            serde_json::json!({"retained": true}),
        )],
    );
    let failure = match outcome.into_result() {
        Err(failure) => failure,
        Ok(()) => panic!("a retained original SQL failure cannot publish success"),
    };
    assert!(matches!(failure.original(), Some(Err("consumer-original"))));
    assert_eq!(
        failure.first_error(),
        Some(financial::PaperInputActionError::Sql)
    );
    assert_eq!(failure.secondary_error_count(), 1);
    assert_eq!(failure.retained_outcome_count(), 1);
    assert!(!failure.commit_started());
    assert!(!failure.committed());

    let outcome = financial::CheckedPaperActionOutcome::new(
        Some(Ok::<(), &'static str>(())),
        Some(financial::PaperInputActionError::CommitUnknown),
        Vec::new(),
        false,
        true,
        Vec::new(),
    );
    let failure = match outcome.into_result() {
        Err(failure) => failure,
        Ok(()) => panic!("a possibly committed action cannot publish ordinary success"),
    };
    assert!(failure.commit_started());
    assert!(matches!(failure.original(), Some(Ok(()))));
}

fn ordinary_fixture(case: Case) -> Fixture {
    Fixture {
        schema: "r5-checker-functional-fixture-v1".to_owned(),
        classification: "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN".to_owned(),
        run_nonce: "1".repeat(32),
        case,
        port: 5432,
        database: format!("r5_checker_{}", "1".repeat(32)),
        worker_password: "2".repeat(64),
        server_version_num: 180004,
        evaluated_at: "2026-09-02T08:00:00Z".parse().expect("fixture date"),
        sessions: ["2026-08-31", "2026-09-01", "2026-09-02"].map(|s| s.parse().expect("date")),
        binding: Value::Null,
    }
}

#[test]
fn fixture_binding_rejects_wrong_case_nonce_database_and_classification() {
    let mut fixture = ordinary_fixture(Case::Normal);
    assert!(validate_fixture(&fixture, &fixture.run_nonce, Case::Normal).is_ok());
    assert!(validate_fixture(&fixture, &fixture.run_nonce, Case::Manual).is_err());
    assert!(validate_fixture(&fixture, &"3".repeat(32), Case::Normal).is_err());
    fixture.database = "postgres".to_owned();
    assert!(validate_fixture(&fixture, &fixture.run_nonce, Case::Normal).is_err());
    fixture = ordinary_fixture(Case::Normal);
    fixture.classification = "PRODUCTION".to_owned();
    assert!(validate_fixture(&fixture, &fixture.run_nonce, Case::Normal).is_err());
}

#[test]
fn verdict_check_cannot_pass_a_negative_on_unrelated_early_refusal() {
    for case in [Case::Manual, Case::Tamper, Case::WrongInput, Case::FailedE] {
        let fixture = ordinary_fixture(case);
        assert!(!matches_expected(
            &fixture,
            &denied(Reason::ReaderIncompatible, fixture.evaluated_at)
        ));
        assert!(matches_expected(
            &fixture,
            &denied(case.reason().expect("negative"), fixture.evaluated_at)
        ));
    }
    let fixture = ordinary_fixture(Case::Normal);
    assert!(!matches_expected(
        &fixture,
        &denied(Reason::OriginUnproven, fixture.evaluated_at)
    ));
}

#[test]
fn explicit_fixture_clock_uses_elapsed_time_and_resets_without_host_change() {
    let at = ordinary_fixture(Case::Normal).evaluated_at;
    assert!(start_override().is_none());
    {
        let _scope = FixtureClock::enter(at).expect("new thread-local scope");
        assert!(FixtureClock::enter(at).is_err());
        let mut clock = EvaluationClock::start().expect("private fixture start");
        assert_eq!(clock.started_at, at);
        assert!(clock.checkpoint().is_ok());
        assert_eq!(
            sample_override(at, Duration::from_secs(2)),
            Some(at + chrono::Duration::seconds(2))
        );
        assert!(
            std::thread::spawn(start_override)
                .join()
                .expect("joined")
                .is_none()
        );
    }
    assert!(start_override().is_none());
}

// These executable construction phases use the existing real public engines.
// Root runs each in its own role namespace and controls the disposable DB clock.
// There is no fixture constructor for Sealed*, Approved*, Current or Report.
#[derive(Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Produce,
    Admit,
    Schedule,
    Compute,
    SourceReserve,
    SourceLinkAdmission,
    SourceLinkRun,
    SourceRead,
    DisabledE,
    PendingE,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingERequest {
    signal: NaiveDate,
    effective: NaiveDate,
    signal_run: uuid::Uuid,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRequest {
    slot: String,
    admission: Option<ContentHash>,
    run: Option<uuid::Uuid>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseFixture {
    schema: String,
    classification: String,
    run_nonce: String,
    phase: Phase,
    daily_config_sha256: ContentHash,
    evaluated_at: DateTime<Utc>,
    port: u16,
    database: String,
    password: String,
    heartbeat_seconds: u64,
    lease_seconds: u64,
    compute_timeout_seconds: u64,
    backoff_seconds: u64,
    #[serde(default)]
    source: Option<SourceRequest>,
    #[serde(default)]
    pending_e: Option<PendingERequest>,
    #[serde(default)]
    e_session: Option<NaiveDate>,
}

fn phase_fixture(phase: Phase) -> PhaseFixture {
    let fixture = any_phase_fixture();
    assert!(fixture.phase == phase, "R5_INTEG_PHASE_SELECTOR");
    fixture
}

fn any_phase_fixture() -> PhaseFixture {
    let (nonce, bytes) = read_fixture_file().unwrap_or_else(|()| panic!("R5_INTEG_PHASE_INPUT"));
    let fixture: PhaseFixture =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("R5_INTEG_PHASE_SCHEMA"));
    assert!(
        fixture.schema == "r5-checker-construction-phase-v1"
            && fixture.classification == "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN"
            && fixture.run_nonce == nonce
            && fixture.database == format!("r5_checker_{nonce}")
            && fixture.port > 0
            && lower_hex(&fixture.password, 64)
            && fixture.heartbeat_seconds > 0
            && fixture.heartbeat_seconds < fixture.lease_seconds
            && fixture.lease_seconds <= 300
            && (1..=600).contains(&fixture.compute_timeout_seconds)
            && fixture.backoff_seconds > 0,
        "R5_INTEG_PHASE_CONTRACT"
    );
    fixture
}

enum SourcePhaseError {
    Policy(wire::EvidenceError),
    Database(sqlx::Error),
    Observed(Box<checked_admission::ObservedAdmissionFailure>),
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-only genuine public Source/observed-admission and disposable role SQL linkage"]
async fn construct_source_sql_link() {
    let fixture = any_phase_fixture();
    assert!(
        matches!(
            fixture.phase,
            Phase::SourceReserve
                | Phase::SourceLinkAdmission
                | Phase::SourceLinkRun
                | Phase::SourceRead
        ),
        "R5_INTEG_SOURCE_PHASE"
    );
    let request = fixture.source.as_ref().expect("explicit source request");
    let admitter = matches!(
        fixture.phase,
        Phase::SourceReserve | Phase::SourceLinkAdmission
    );
    let pool = phase_pool(
        &fixture,
        if admitter {
            "daily_input_admitter"
        } else {
            "worker"
        },
    )
    .await;
    let mut observations = Vec::new();
    let result=async {
        let source=ProtectedSourceChain::open(Path::new(SOURCE_EVENTS_ROOT),&request.slot).map_err(SourcePhaseError::Policy)?;
        let first=source.first();
        let session=NaiveDate::parse_from_str(first["session"].as_str().ok_or(SourcePhaseError::Policy(wire::EvidenceError))?,"%Y-%m-%d")
            .map_err(|_|SourcePhaseError::Policy(wire::EvidenceError))?;
        let mut tx=pool.begin().await.map_err(SourcePhaseError::Database)?;
        let result:Value=match fixture.phase {
            Phase::SourceReserve=>{
                let reservation=uuid::Uuid::parse_str(first["reservation_id"].as_str().ok_or(SourcePhaseError::Policy(wire::EvidenceError))?)
                    .map_err(|_|SourcePhaseError::Policy(wire::EvidenceError))?;
                let root=wire::digest(&wire::canonical(source.event("RESERVED").map_err(SourcePhaseError::Policy)?).map_err(SourcePhaseError::Policy)?);
                sqlx::query_scalar("SELECT public.reserve_owner_beta_paper_source_slot($1,$2,$3,$4,$5)")
                    .bind(&request.slot).bind(reservation).bind(root).bind(&first["cohort"]).bind(session)
                    .fetch_one(&mut *tx).await.map_err(SourcePhaseError::Database)?
            }
            Phase::SourceLinkAdmission=>{
                let admission=request.admission.as_ref().ok_or(SourcePhaseError::Policy(wire::EvidenceError))?;
                let row:(String,String,String,String)=sqlx::query_as("SELECT content_sha256,manifest_sha256,admission_sha256,policy_sha256 FROM public.read_owner_beta_daily_admitted_input($1) WHERE usable")
                    .bind(admission.as_str()).fetch_one(&mut *tx).await.map_err(SourcePhaseError::Database)?;
                let hash=|value:&str|ContentHash::parse(value).map_err(|_|SourcePhaseError::Policy(wire::EvidenceError));
                let pins=market_data::daily_price_only::DailyPricePinsV1Wire{content_sha256:hash(&row.0)?,manifest_sha256:hash(&row.1)?,admission_sha256:hash(&row.2)?,policy_sha256:hash(&row.3)?};
                observations.push(checked_admission::verify_admission_observed(&source,&pins)
                    .map_err(|failure|SourcePhaseError::Observed(Box::new(failure)))?);
                let event=source.event("SOURCE_FINISHED").map_err(SourcePhaseError::Policy)?;
                let event_hash=wire::digest(&wire::canonical(event).map_err(SourcePhaseError::Policy)?);
                let originals=event["data"]["original_set_sha256"].as_str().ok_or(SourcePhaseError::Policy(wire::EvidenceError))?;
                sqlx::query_scalar("SELECT public.link_owner_beta_paper_source_admission($1,$2,$3,$4)")
                    .bind(&request.slot).bind(event_hash).bind(originals).bind(admission.as_str())
                    .fetch_one(&mut *tx).await.map_err(SourcePhaseError::Database)?
            }
            Phase::SourceLinkRun=>sqlx::query_scalar("SELECT public.link_owner_beta_paper_daily_run($1,$2)")
                .bind(&request.slot).bind(request.run.ok_or(SourcePhaseError::Policy(wire::EvidenceError))?)
                .fetch_one(&mut *tx).await.map_err(SourcePhaseError::Database)?,
            Phase::SourceRead=>sqlx::query_scalar("SELECT public.read_owner_beta_paper_session_evidence($1,$2)")
                .bind(session).bind(&first["cohort"]).fetch_one(&mut *tx).await.map_err(SourcePhaseError::Database)?,
            _=>unreachable!("phase preflight"),
        };
        source.revalidate().map_err(SourcePhaseError::Policy)?;
        tx.commit().await.map_err(SourcePhaseError::Database)?;
        Ok::<_,SourcePhaseError>(result)
    }.await;
    pool.close().await;
    // Original failures and successful observed bags survive owned pool closure.
    let value = match result {
        Ok(value) => value,
        Err(SourcePhaseError::Policy(source)) => {
            drop((source, observations));
            panic!("R5_INTEG_SOURCE_POLICY");
        }
        Err(SourcePhaseError::Database(source)) => {
            drop((source, observations));
            panic!("R5_INTEG_SOURCE_SQL");
        }
        Err(SourcePhaseError::Observed(source)) => {
            drop((source, observations));
            panic!("R5_INTEG_SOURCE_OBSERVATION");
        }
    };
    let path = PathBuf::from(format!(
        "{FIXTURE_ROOT}/{}/phase-result.json",
        fixture.run_nonce
    ));
    write_fixture_output(
        &path,
        &wire::canonical(&value).expect("actual SQL output encoding"),
    );
    println!(
        "R5_INTEG phase=source_sql actual_public_sql=true origin=UNKNOWN pool_close_awaited=true"
    );
}

async fn phase_pool(fixture: &PhaseFixture, role: &str) -> PgPool {
    let options = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(fixture.port)
        .database(&fixture.database)
        .username(role)
        .password(&fixture.password)
        .ssl_mode(PgSslMode::Disable);
    let bounded_e_helper = matches!(fixture.phase, Phase::DisabledE | Phase::PendingE);
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if bounded_e_helper {
                    sqlx::raw_sql("SET statement_timeout='15s'; SET lock_timeout='5s'")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .unwrap_or_else(|_| panic!("R5_INTEG_PHASE_CONNECT"));
    let identity:Result<PhaseIdentity,sqlx::Error>=sqlx::query_as(
        "SELECT current_database()::text,current_user::text,session_user::text,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls,\
         (SELECT count(*) FROM public.r5_checker_fixture_identity),(SELECT min(nonce) FROM public.r5_checker_fixture_identity)\
         FROM pg_roles WHERE rolname=current_user").fetch_one(&pool).await;
    let valid = identity.as_ref().is_ok_and(|r| {
        r.0 == fixture.database
            && r.1 == role
            && r.2 == role
            && !r.3
            && !r.4
            && !r.5
            && !r.6
            && !r.7
            && r.8 == 1
            && r.9.as_deref() == Some(fixture.run_nonce.as_str())
    });
    if !valid {
        pool.close().await;
        drop(identity);
        panic!("R5_INTEG_PHASE_IDENTITY");
    }
    pool
}

fn phase_config(fixture: &PhaseFixture) -> DailyIngestConfig {
    DailyIngestConfig::from_pinned_file(Path::new(DAILY_CONFIG_ALIAS), &fixture.daily_config_sha256)
        .unwrap_or_else(|_| panic!("R5_INTEG_PHASE_CONFIG"))
}

#[test]
#[ignore = "Root-only isolated UID10002 producer phase, genuine approved base RO"]
fn construct_daily_candidate() {
    use crate::owner_beta::runner::{
        daily_ingest::{IngestOutcome, produce_daily_once},
        daily_runtime::DailyProcessMode,
    };
    let fixture = phase_fixture(Phase::Produce);
    let config = phase_config(&fixture);
    let outcome = produce_daily_once(
        &config,
        DailyProcessMode::OwnerOnly,
        fixture.evaluated_at,
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert!(
        matches!(outcome, Ok(IngestOutcome::CandidateReady)),
        "R5_INTEG_CANDIDATE_NOT_READY outcome={outcome:?}"
    );
    println!("R5_INTEG phase=produce actual_candidate_builder=true");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-only isolated UID10003 + daily_input_admitter phase, genuine base RO"]
async fn construct_daily_admission() {
    use crate::owner_beta::runner::{
        daily_ingest::{IngestOutcome, admit_daily_once},
        daily_runtime::DailyProcessMode,
    };
    let fixture = phase_fixture(Phase::Admit);
    let config = phase_config(&fixture);
    let pool = phase_pool(&fixture, "daily_input_admitter").await;
    let outcome = admit_daily_once(
        &pool,
        &config,
        DailyProcessMode::OwnerOnly,
        fixture.evaluated_at,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await;
    pool.close().await;
    assert!(
        matches!(outcome, Ok(IngestOutcome::Admitted)),
        "R5_INTEG_ADMISSION_NOT_WRITTEN outcome={outcome:?}"
    );
    println!("R5_INTEG phase=admit actual_verification_and_sealing=true");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Root-only disposable worker SQL schedule phase"]
async fn reserve_daily_computation() {
    use crate::owner_beta::runner::{
        daily_runtime::DailyProcessMode,
        daily_schedule::{DailyScheduleOutcome, schedule_daily_once},
    };
    let fixture = phase_fixture(Phase::Schedule);
    let pool = phase_pool(&fixture, "worker").await;
    let outcome = schedule_daily_once(&pool, DailyProcessMode::OwnerOnly).await;
    pool.close().await;
    assert!(
        matches!(outcome,Ok(DailyScheduleOutcome::Completed(r)) if r.checked==1 && r.reserved==1 && r.replayed==0 && r.deferred==0 && r.blocked==0),
        "R5_INTEG_RUN_NOT_RESERVED"
    );
    println!("R5_INTEG phase=schedule actual_role_reservation=true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Root-only real sealed Daily computation/publication/queue settlement"]
async fn compute_and_publish_daily() {
    use crate::owner_beta::runner::{
        OwnerBetaOutcome, OwnerBetaRunnerConfig, OwnerBetaRunnerPaths,
        daily_execution::{DailyExecutionContext, DailyExecutionMode, run_once_with_daily},
    };
    let fixture = phase_fixture(Phase::Compute);
    let config = phase_config(&fixture);
    let pool = phase_pool(&fixture, "worker").await;
    let runner = OwnerBetaRunnerConfig::new(
        Duration::from_secs(fixture.heartbeat_seconds),
        Duration::from_secs(fixture.lease_seconds),
        Duration::from_secs(fixture.compute_timeout_seconds),
    )
    .expect("validated runner durations");
    let queue = crate::JobQueue::new(
        pool.clone(),
        None,
        crate::QueueConfig {
            lease: runner.lease(),
            backoff_base: Duration::from_secs(fixture.backoff_seconds),
        },
    );
    let daily = DailyExecutionMode::OwnerOnly(DailyExecutionContext {
        admission_root: config.admission_path(),
        trusted_owner: DAILY_ADMITTER_UID,
        reader_group: DAILY_READER_GROUP,
        policy: config.policy(),
        replay_limits: config.replay_limits(),
    });
    let outcome = run_once_with_daily(
        &pool,
        &queue,
        &format!("integ1-{}", fixture.run_nonce),
        &OwnerBetaRunnerPaths {
            artifact_root: config.historical_path().to_owned(),
        },
        &runner,
        &daily,
    )
    .await;
    drop(queue);
    pool.close().await;
    match outcome {
        Ok(OwnerBetaOutcome::Succeeded { job_id, run_id }) => println!(
            "R5_INTEG phase=compute actual_computation_publication=true job_id={job_id} run_id={run_id}"
        ),
        _ => panic!("R5_INTEG_REAL_COMPUTATION_DID_NOT_SUCCEED"),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparationFixture {
    schema: String,
    classification: String,
    run_nonce: String,
    policy: Value,
    config: Value,
}

fn canonical_policy_and_config(policy: &Value, config: &Value) -> Result<(Vec<u8>, Vec<u8>), ()> {
    use market_data::daily_price_only::{
        encode_daily_price_policy_v1, parse_daily_price_policy_v1,
    };
    let policy = parse_daily_price_policy_v1(&serde_json::to_vec(policy).map_err(|_| ())?)
        .map_err(|_| ())?;
    let bytes = encode_daily_price_policy_v1(&policy).map_err(|_| ())?;
    let mut config = config.clone();
    let fields = config.as_object_mut().ok_or(())?;
    if fields.get("schema").and_then(Value::as_str) != Some("owner-beta-daily-process-v1")
        || fields.get("policy_path").and_then(Value::as_str) != Some(DAILY_POLICY_ALIAS)
        || fields.contains_key("policy_sha256")
    {
        return Err(());
    }
    fields.insert(
        "policy_sha256".to_owned(),
        serde_json::to_value(ContentHash::from_bytes(&bytes)).map_err(|_| ())?,
    );
    Ok((bytes, serde_json::to_vec(&config).map_err(|_| ())?))
}

fn write_fixture_output(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .unwrap_or_else(|_| panic!("R5_INTEG_OUTPUT_CREATE"));
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .unwrap_or_else(|_| panic!("R5_INTEG_OUTPUT_WRITE"));
}

#[test]
#[ignore = "Root-only private fixture preparation; ordinary config bytes grant no approval"]
fn prepare_daily_configuration() {
    prepare_daily_configuration_with_strategy(crate::resolver::ResolvedConfig {
        strategy_id: "buy_and_hold".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: serde_json::json!({"benchmark_instrument":"069500.KRX","target_weight":1.0}),
    });
}

#[test]
#[ignore = "Root-only private R5-03 adopted-strategy fixture preparation"]
fn prepare_paper_daily_configuration() {
    prepare_daily_configuration_with_strategy(crate::resolver::ResolvedConfig {
        strategy_id: "trend_following".to_owned(),
        strategy_version: "1.0.0".to_owned(),
        config: serde_json::json!({"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}),
    });
}

fn prepare_daily_configuration_with_strategy(strategy: crate::resolver::ResolvedConfig) {
    let (nonce, bytes) = read_fixture_file().expect("private preparation input");
    let fixture: PreparationFixture = serde_json::from_slice(&bytes).expect("preparation shape");
    assert!(
        fixture.schema == "r5-checker-prepare-v1"
            && fixture.run_nonce == nonce
            && fixture.classification == "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN",
        "R5_INTEG_PREPARATION_BINDING"
    );
    let (policy, config) = canonical_policy_and_config(&fixture.policy, &fixture.config)
        .expect("actual policy parser and encoder");
    let parent = PathBuf::from(format!("{FIXTURE_ROOT}/{nonce}/prepared"));
    std::fs::create_dir(&parent).expect("fresh private output");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
        .expect("private output mode");
    write_fixture_output(&parent.join("policy.json"), &policy);
    write_fixture_output(&parent.join("config.json"), &config);
    let result = serde_json::json!({"classification":"SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN",
        "policy_sha256":ContentHash::from_bytes(&policy),"config_sha256":ContentHash::from_bytes(&config),
        "strategy":crate::owner_beta::OwnerBetaStrategySnapshot::from_resolved_config(&strategy)
            .expect("ordinary actual strategy snapshot"),
        "approval_created":false,"actual_policy_parser":true});
    write_fixture_output(
        &parent.join("pins.json"),
        &serde_json::to_vec(&result).expect("ordinary pins"),
    );
    println!("R5_INTEG phase=prepare actual_policy_parser=true approval_created=false");
}

// Appended isolated selectors: no existing phase/clock/diagnostic is replaced.
struct PendingEWork {
    pool: PgPool,
    request: PendingERequest,
    run: uuid::Uuid,
    job: uuid::Uuid,
    attempt: uuid::Uuid,
}
impl crate::owner_beta::paper::action::PaperActionWork<(), &'static str> for PendingEWork {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx crate::owner_beta::paper::action::PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> crate::owner_beta::paper::action::PaperActionFuture<'ctx, (), &'static str> {
        Box::pin(async move {
            let selector = financial::PaperActionInputRequest::Settle {
                signal: trading_date(self.request.signal),
                effective: trading_date(self.request.effective),
                run_id: self.request.signal_run,
            };
            let outcome = financial::with_checked_paper_action(
                &self.pool,
                context,
                &selector,
                PendingEConsumer {
                    run: self.run,
                    job: self.job,
                    attempt: self.attempt,
                },
            )
            .await;
            let result = outcome.into_result();
            match &result {
                Ok(()) => Ok(()),
                Err(_) => Err("PENDING_E_CHECKED_SCOPE_REFUSED"),
            }
            // The whole actual lower result remains alive through scope finish
            // and this last local mapping; it is never a transported capability.
        })
    }
}
struct PendingEConsumer {
    run: uuid::Uuid,
    job: uuid::Uuid,
    attempt: uuid::Uuid,
}
impl financial::PaperFinancialConsumer<(), &'static str> for PendingEConsumer {
    fn run<'a, 'hold>(
        self,
        action: &'a mut financial::CheckedPaperAction<'hold>,
    ) -> financial::LocalInputFuture<'a, Result<(), &'static str>>
    where
        Self: 'a,
        'hold: 'a,
    {
        Box::pin(async move {
            let e = action
                .inputs()
                .execution()
                .ok_or("PENDING_E_PRICE_MISSING")?;
            let wire = e.readback().as_untrusted_value();
            if wire["candidate_run_id"] != self.run.to_string()
                || wire["job_id"] != self.job.to_string()
                || wire["attempt_id"] != self.attempt.to_string()
                || wire["terminal"] != "RUNNING"
                || !wire["finished_at"].is_null()
                || !wire["factor_snapshot_sha256"].is_null()
                || !wire["target_snapshot_sha256"].is_null()
                || wire["admission"].is_null()
                || wire["candidate_pins"].is_null()
            {
                return Err("PENDING_E_ACTUAL_WIRE_MISMATCH");
            }
            action
                .begin_transaction()
                .await
                .map_err(|_| "PENDING_E_BEGIN")?;
            let original = sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&mut **action.transaction().map_err(|_| "PENDING_E_TX")?)
                .await;
            let value = match original {
                Ok(value) => {
                    action.retain_value(serde_json::json!(value));
                    value
                }
                Err(error) => {
                    action.retain_sql_error(error);
                    let _ = action.rollback().await;
                    return Err("PENDING_E_SELECT");
                }
            };
            let rollback = action.rollback().await;
            if value != 1 {
                return Err("PENDING_E_SELECT_VALUE");
            }
            rollback.map_err(|_| "PENDING_E_ROLLBACK")?;
            eprintln!(
                "R5_ACTION_PENDING_E actual_checked_scope=true actual_SQL_finished_at_NULL=true harmless_SELECT=true rollback_awaited=true"
            );
            Ok(())
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Root-only real reserved E; disabled runner or pending checked scope, then canonical failure; never host execution"]
async fn actual_daily_disabled_or_pending_e() {
    use crate::owner_beta::{
        OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaPublicationFailure,
        OwnerBetaPublicationOutcome,
        daily_input::OwnerBetaExecutionInput,
        publish::settle_owner_beta_daily_failure,
        runner::{
            OwnerBetaOutcome, OwnerBetaRunnerConfig, OwnerBetaRunnerPaths,
            daily_execution::{DailyExecutionMode, run_once_with_daily},
        },
    };
    let fixture = any_phase_fixture();
    assert!(
        matches!(fixture.phase, Phase::DisabledE | Phase::PendingE),
        "R5_E_HELPER_PHASE"
    );
    let effective = fixture.e_session.expect("explicit actual reserved E date");
    if let Some(request) = &fixture.pending_e {
        assert!(
            request.effective == effective
                && request.signal < effective
                && !request.signal_run.is_nil(),
            "R5_PENDING_E_SELECTORS"
        );
    }
    assert!(
        (fixture.phase == Phase::PendingE) == fixture.pending_e.is_some(),
        "R5_E_PHASE_SELECTORS"
    );
    let config = phase_config(&fixture);
    let pool = phase_pool(&fixture, "worker").await;
    // A distinct real connection allows heartbeat while the checked reader owns
    // its one economic/readback transaction. The ordinary queue API owns claims.
    let monitor_pool = phase_pool(&fixture, "worker").await;
    let runner = OwnerBetaRunnerConfig::new(
        Duration::from_secs(fixture.heartbeat_seconds),
        Duration::from_secs(fixture.lease_seconds),
        Duration::from_secs(fixture.compute_timeout_seconds),
    )
    .unwrap();
    let queue = crate::JobQueue::new(
        monitor_pool.clone(),
        None,
        crate::QueueConfig {
            lease: runner.lease(),
            backoff_base: Duration::from_secs(fixture.backoff_seconds),
        },
    );
    // Refuse before claiming unless the isolated queue contains exactly the
    // genuine queued Daily E selected by the existing reservation phase. In
    // particular Disabled must never fall through into Historical computation.
    let queued: Result<Vec<(uuid::Uuid, serde_json::Value)>, sqlx::Error> = sqlx::query_as(
        "SELECT id,payload_json FROM public.jobs WHERE job_type=$1 AND status='QUEUED' ORDER BY id",
    )
    .bind(OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE)
    .fetch_all(&pool)
    .await;
    let selected = queued
        .as_ref()
        .ok()
        .and_then(|rows| (rows.len() == 1).then(|| &rows[0]));
    let selected_input = selected
        .map(|(_, payload)| serde_json::from_value::<OwnerBetaExecutionInput>(payload.clone()));
    let selected_ids = match (selected, selected_input.as_ref()) {
        (Some((job, _)), Some(Ok(OwnerBetaExecutionInput::Daily(input))))
            if input.as_of().as_naive_date() == effective =>
        {
            Some((*job, input.run_id()))
        }
        _ => None,
    };
    let selected_ids = match selected_ids {
        Some(ids) => ids,
        None => {
            drop(queue);
            monitor_pool.close().await;
            pool.close().await;
            panic!("R5_E_EXACT_ONE_RESERVED_DAILY_REQUIRED");
        }
    };
    let mut disabled_original = None;
    let mut pending_original = None;
    let mut pending_cleanup = None;
    let mut monitor_result = None;
    let (job, run) = if fixture.phase == Phase::DisabledE {
        // reserve_daily_computation is the preceding existing selector, not a
        // hidden UPDATE/retry. Disabled is the genuine runner dispatch branch.
        let original = run_once_with_daily(
            &pool,
            &queue,
            &format!("disabled-e-{}", fixture.run_nonce),
            &OwnerBetaRunnerPaths {
                artifact_root: config.historical_path().to_owned(),
            },
            &runner,
            &DailyExecutionMode::Disabled,
        )
        .await;
        let ids = match &original {
            Ok(OwnerBetaOutcome::Blocked {
                job_id,
                run_id,
                code,
            }) if *code == OwnerBetaPublicationFailure::DailyExecutionDisabled.code() => {
                (*job_id, *run_id)
            }
            _ => {
                drop(queue);
                monitor_pool.close().await;
                pool.close().await;
                panic!("R5_DISABLED_E_REAL_RUNNER_DID_NOT_BLOCK");
            }
        };
        disabled_original = Some(original);
        ids
    } else {
        let request = fixture.pending_e.expect("explicit pending E/T selectors");
        let returned_claim = queue
            .claim_next_for(
                &format!("pending-e-{}", fixture.run_nonce),
                OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
            )
            .await;
        let claim = match returned_claim {
            Ok(Some(claim)) => claim,
            original => {
                drop(queue);
                monitor_pool.close().await;
                pool.close().await;
                let _ = &original;
                panic!("R5_PENDING_E_REAL_CLAIM_UNAVAILABLE");
            }
        };
        let parsed =
            serde_json::from_value::<OwnerBetaExecutionInput>(claim.job.payload_json.clone());
        let input = match &parsed {
            Ok(OwnerBetaExecutionInput::Daily(input)) => input,
            _ => {
                // This fixture owns the real claim; malformed input is terminally
                // refused through the queue API rather than leaving a live claim.
                let cleanup = queue
                    .settle_failure(
                        &claim,
                        crate::ErrorClass::Integrity,
                        "R5_PENDING_E_PAYLOAD_INVALID",
                        "isolated pending E payload refused",
                    )
                    .await;
                drop(queue);
                monitor_pool.close().await;
                pool.close().await;
                assert!(cleanup.is_ok(), "R5_PENDING_E_MALFORMED_CLEANUP");
                panic!("R5_PENDING_E_NOT_DAILY");
            }
        };
        let same_effective = input.as_of().as_naive_date() == request.effective;
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let actual_queue = queue.clone();
        let actual_claim = claim.clone();
        let period = Duration::from_secs(fixture.heartbeat_seconds);
        let monitor = tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = stopped.changed() => { if changed.is_err() || *stopped.borrow() { return None; } },
                    _ = tokio::time::sleep(period) => {
                        let returned = actual_queue.heartbeat(&actual_claim).await;
                        if !matches!(&returned, Ok(crate::HeartbeatStatus::Extended)) { return Some(returned); }
                    }
                }
            }
        });
        let original = crate::owner_beta::paper::action::run_bound_paper_action(
            crate::owner_beta::paper::runtime::PaperProcessRole::Worker,
            Duration::from_secs(90),
            None,
            PendingEWork {
                pool: pool.clone(),
                request,
                run: input.run_id(),
                job: claim.job.id,
                attempt: claim.attempt.id,
            },
        )
        .await;
        // Finish even when the probe failed; hold the whole original through
        // actual settlement and monitor shutdown, then inspect both separately.
        let cleanup = settle_owner_beta_daily_failure(
            &pool,
            &queue,
            &claim,
            input,
            OwnerBetaPublicationFailure::DailyExecutionDisabled,
        )
        .await;
        let _ = stop.send(true);
        let monitored = monitor.await;
        let ids = (claim.job.id, input.run_id());
        pending_original = Some((original, same_effective));
        pending_cleanup = Some(cleanup);
        monitor_result = Some(monitored);
        ids
    };
    let readback = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid, i32, String, String, String, bool, bool, bool, bool)>(
        "SELECT j.id,r.id,a.id,a.attempt_no,j.status::text,r.status::text,a.outcome::text, \
         j.finished_at IS NOT NULL,r.finished_at IS NOT NULL,a.finished_at IS NOT NULL, \
         r.factor_snapshot_sha256 IS NULL AND r.target_snapshot_sha256 IS NULL \
         FROM public.jobs j JOIN public.owner_beta_recommendation_runs r ON r.job_id=j.id \
         JOIN public.job_attempts a ON a.job_id=j.id AND a.attempt_no=j.attempt_count WHERE j.id=$1 AND r.id=$2")
        .bind(job).bind(run).fetch_one(&pool).await;
    drop(queue);
    monitor_pool.close().await;
    pool.close().await;
    if let Some((original, same_effective)) = &pending_original {
        assert!(
            *same_effective && original.is_ok(),
            "R5_PENDING_E_REAL_SCOPE_FAILED"
        );
    }
    if let Some(cleanup) = &pending_cleanup {
        assert!(
            matches!(cleanup, Ok(OwnerBetaPublicationOutcome::Failed)),
            "R5_PENDING_E_FAILURE_API_FAILED"
        );
    }
    if let Some(monitored) = &monitor_result {
        assert!(
            matches!(
                monitored,
                Ok(None) | Ok(Some(Ok(crate::HeartbeatStatus::LeaseLost)))
            ),
            "R5_PENDING_E_MONITOR_FAILED"
        );
    }
    assert_eq!(
        (job, run),
        selected_ids,
        "the exact preflight-selected actual reserved E must be retired"
    );
    let _ = (&disabled_original, &queued, &selected_input);
    let row = readback.unwrap_or_else(|_| panic!("R5_E_TERMINAL_READBACK_FAILED"));
    assert_eq!((row.0, row.1), (job, run));
    assert_eq!(row.3, 1);
    assert_eq!((&*row.4, &*row.5, &*row.6), ("FAILED", "FAILED", "FAILED"));
    assert!(row.7 && row.8 && row.9 && row.10);
    eprintln!(
        "R5_E_HELPER actual_claim=true job_id={} run_id={} attempt_id={} attempt_no=1 terminal_FAILED=true no_FW=true never_retried_as_T=true",
        row.0, row.1, row.2
    );
}
