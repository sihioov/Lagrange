//! Operator-gated Raw-only capture for the fixed 30-stock price beta.
//!
//! The default action is a local plan. Only `--execute` together with the
//! exact acknowledgement constructs the credentialed KIS read client. This
//! binary commits no normalized data, artifact, database, or publication.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use collectors::stock_price_beta_raw::{
    CaptureError, CaptureIdentity, CaptureOutcome, CaptureWindow, DAILY_BARS_PATH,
    DAILY_BARS_TR_ID, ENTITLEMENT_DOCUMENT_REFERENCE, ENTITLEMENT_FILE_SHA256, ENTITLEMENT_ID,
    ENTITLEMENT_PROVIDER, FID_ORG_ADJ_PRC, FIXED_CAPTURE_WINDOWS, FIXED_RANGE_END,
    FIXED_RANGE_START, FIXED_SYMBOL_COUNT, FIXED_UNIVERSE_FILE_SHA256, MAX_PLANNED_GETS,
    MAX_WINDOWS_PER_SYMBOL, RAW_CONTRACT_VERSION, RAW_INTERVAL, RAW_MARKET, RAW_PROVIDER,
    capture_raw, load_entitlement, load_universe, validate_entitlement_binding,
};
use domain::{ContentHash, TradingDate};
use kis_client::clock::Clock;
use kis_client::live_transport::LiveTransport;
use kis_client::secret::SystemCredentialSource;
use kis_client::token_issuer::KisTokenIssuer;
use kis_client::{
    BucketKey, CredentialRef, KisMarketDataClient, Quota, RateLimiter, SystemClock, TokenManager,
    TokioSleeper,
};
use market_data::storage::RawStore;
use serde::Serialize;

const DEFAULT_START: &str = FIXED_RANGE_START;
const DEFAULT_END: &str = FIXED_RANGE_END;
const DEFAULT_UNIVERSE_RELATIVE_PATH: &str = "configs/universes/kr-stock-price-beta-v1.json";
const IMAGE_UNIVERSE_PATH: &str = "/opt/lagrange/configs/universes/kr-stock-price-beta-v1.json";
const DEFAULT_ENTITLEMENT_RELATIVE_PATH: &str = "configs/data-rights/kis.entitlement.json";
const IMAGE_ENTITLEMENT_PATH: &str = "/opt/lagrange/configs/data-rights/kis.entitlement.json";
const RAW_ROOT_ENV: &str = "RESEARCH_RAW_ROOT";
const ENTITLEMENT_REFERENCE_ENV: &str = "RESEARCH_ENTITLEMENT_REFERENCE";
const ENTITLEMENT_HASH_ENV: &str = "RESEARCH_ENTITLEMENT_SHA256";
const APP_KEY_FILE_ENV: &str = "KIS_APP_KEY_FILE";
const APP_SECRET_FILE_ENV: &str = "KIS_APP_SECRET_FILE";
const COMMIT_ENV: &str = "LAGRANGE_CODE_COMMIT";
const CONFIRM_ENV: &str = "KIS_STOCK_PRICE_BETA_CONFIRM";
const CONFIRM_VALUE: &str = "I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS";
const KIS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

type LiveKisClient = KisMarketDataClient<LiveTransport, TokioSleeper, SystemCredentialSource>;

const USAGE: &str = "\
kis-stock-price-beta-raw [options]\n\
\n\
Options:\n\
  --start YYYY-MM-DD       fixed: 2025-08-04\n\
  --end YYYY-MM-DD         fixed: 2026-08-28\n\
  --plan                   print the local request plan (default)\n\
  --check                  validate local paths and contract identity\n\
  --execute                perform the acknowledged credentialed Raw capture\n\
  --help\n\
\n\
Execute-only environment:\n\
  configs/data-rights/kis.entitlement.json is pinned by its exact checked-in bytes\n\
  RESEARCH_RAW_ROOT\n\
  RESEARCH_ENTITLEMENT_REFERENCE  must match the checked-in document reference\n\
  RESEARCH_ENTITLEMENT_SHA256  must match the checked-in entitlement file bytes\n\
  KIS_APP_KEY_FILE\n\
  KIS_APP_SECRET_FILE\n\
  LAGRANGE_CODE_COMMIT  exact 40-character lowercase Git commit\n\
  KIS_STOCK_PRICE_BETA_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_STOCK_PRICE_BETA_CALLS\n\
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Check,
    Execute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Args {
    start: TradingDate,
    end: TradingDate,
    action: Action,
}

#[derive(Debug)]
struct LocalConfig {
    raw_root: PathBuf,
    universe: collectors::stock_price_beta_raw::StockPriceBetaUniverse,
    identity: CaptureIdentity,
    app_key_file: PathBuf,
    app_secret_file: PathBuf,
}

#[derive(Debug)]
enum CliError {
    Usage,
    InvalidDate,
    InvalidRange,
    FixedRangeRequired,
    ConflictingAction,
    MissingConfiguration,
    InvalidConfiguration,
    ConfirmationRequired,
    TransportConfiguration,
    Capture(CaptureError),
}

#[derive(Serialize)]
struct PlanRecord {
    status: &'static str,
    contract_version: &'static str,
    universe: &'static str,
    symbols: usize,
    start: String,
    end: String,
    interval: &'static str,
    endpoint: &'static str,
    tr_id: &'static str,
    fid_org_adj_prc: &'static str,
    max_windows_per_symbol: usize,
    planned_gets_before_retries: usize,
    network_calls: &'static str,
    raw_scope: &'static str,
    publication: &'static str,
    universe_sha256: &'static str,
    entitlement_id: &'static str,
    entitlement_provider: &'static str,
    entitlement_reference: &'static str,
    entitlement_file_sha256: &'static str,
    entitlement_dataset: &'static str,
    entitlement_user: &'static str,
    windows: [WindowRecord; MAX_WINDOWS_PER_SYMBOL],
}

#[derive(Serialize)]
struct WindowRecord {
    id: &'static str,
    start: &'static str,
    end: &'static str,
}

#[derive(Serialize)]
struct CheckRecord {
    status: &'static str,
    contract_version: &'static str,
    universe: &'static str,
    universe_sha256: String,
    start: String,
    end: String,
    batch_id: String,
    planned_gets_before_retries: usize,
    local_only: bool,
    raw_scope: &'static str,
    entitlement_id: &'static str,
    entitlement_provider: &'static str,
    entitlement_reference: &'static str,
    entitlement_file_sha256: &'static str,
}

#[derive(Serialize)]
struct SuccessRecord {
    status: &'static str,
    provider: &'static str,
    market: &'static str,
    contract_version: &'static str,
    start: String,
    end: String,
    batch_id: String,
    planned_gets_before_retries: usize,
    actual_gets_before_retries: usize,
    file_count: usize,
    raw_only: bool,
    normalized: bool,
    publication: bool,
}

#[derive(Serialize)]
struct FailureRecord {
    status: &'static str,
    error_code: &'static str,
    raw_visibility: &'static str,
    network_calls: &'static str,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let values = std::env::args().skip(1).collect::<Vec<_>>();
    let args = match parse_args(&values) {
        Ok(Some(args)) => args,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => return report_parse_error(error),
    };

    let result = match args.action {
        Action::Plan => {
            print_plan(args);
            Ok(())
        }
        Action::Check => run_check(args),
        Action::Execute => run_execute(args).await,
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_error(args, error),
    }
}

fn parse_args(values: &[String]) -> Result<Option<Args>, CliError> {
    if values
        .iter()
        .any(|value| value == "--help" || value == "-h")
    {
        return Ok(None);
    }
    let mut start = TradingDate::parse(DEFAULT_START).map_err(|_| CliError::InvalidDate)?;
    let mut end = TradingDate::parse(DEFAULT_END).map_err(|_| CliError::InvalidDate)?;
    let mut action = None;
    let mut index = 0;
    while index < values.len() {
        match values[index].as_str() {
            "--start" => {
                index += 1;
                start = parse_date(values.get(index))?;
            }
            "--end" => {
                index += 1;
                end = parse_date(values.get(index))?;
            }
            "--plan" => set_action(&mut action, Action::Plan)?,
            "--check" => set_action(&mut action, Action::Check)?,
            "--execute" => set_action(&mut action, Action::Execute)?,
            _ => return Err(CliError::Usage),
        }
        index += 1;
    }
    if end < start {
        return Err(CliError::InvalidRange);
    }
    if start.to_iso() != DEFAULT_START || end.to_iso() != DEFAULT_END {
        return Err(CliError::FixedRangeRequired);
    }
    Ok(Some(Args {
        start,
        end,
        action: action.unwrap_or(Action::Plan),
    }))
}

fn parse_date(value: Option<&String>) -> Result<TradingDate, CliError> {
    value
        .and_then(|value| TradingDate::parse(value).ok())
        .ok_or(CliError::InvalidDate)
}

fn set_action(action: &mut Option<Action>, next: Action) -> Result<(), CliError> {
    if action.replace(next).is_some() {
        return Err(CliError::ConflictingAction);
    }
    Ok(())
}

fn print_plan(args: Args) {
    print_json(&PlanRecord {
        status: "plan",
        contract_version: RAW_CONTRACT_VERSION,
        universe: "kr-stock-price-beta-v1",
        symbols: FIXED_SYMBOL_COUNT,
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        interval: RAW_INTERVAL,
        endpoint: DAILY_BARS_PATH,
        tr_id: DAILY_BARS_TR_ID,
        fid_org_adj_prc: FID_ORG_ADJ_PRC,
        max_windows_per_symbol: MAX_WINDOWS_PER_SYMBOL,
        planned_gets_before_retries: MAX_PLANNED_GETS,
        network_calls: "none",
        raw_scope: RAW_PROVIDER,
        publication: "unsupported",
        universe_sha256: FIXED_UNIVERSE_FILE_SHA256,
        entitlement_id: ENTITLEMENT_ID,
        entitlement_provider: ENTITLEMENT_PROVIDER,
        entitlement_reference: ENTITLEMENT_DOCUMENT_REFERENCE,
        entitlement_file_sha256: ENTITLEMENT_FILE_SHA256,
        entitlement_dataset: "krx_eod_bars",
        entitlement_user: "usr_owner",
        windows: FIXED_CAPTURE_WINDOWS.map(window_record),
    });
}

fn window_record(window: CaptureWindow) -> WindowRecord {
    WindowRecord {
        id: window.id,
        start: window.start,
        end: window.end,
    }
}

fn run_check(args: Args) -> Result<(), CliError> {
    let config = read_local_config(false, args)?;
    print_json(&CheckRecord {
        status: "check",
        contract_version: RAW_CONTRACT_VERSION,
        universe: "kr-stock-price-beta-v1",
        universe_sha256: config.universe.file_sha256().as_str().to_owned(),
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        batch_id: config.identity.batch_id().to_string(),
        planned_gets_before_retries: MAX_PLANNED_GETS,
        local_only: true,
        raw_scope: RAW_PROVIDER,
        entitlement_id: ENTITLEMENT_ID,
        entitlement_provider: ENTITLEMENT_PROVIDER,
        entitlement_reference: ENTITLEMENT_DOCUMENT_REFERENCE,
        entitlement_file_sha256: ENTITLEMENT_FILE_SHA256,
    });
    Ok(())
}

async fn run_execute(args: Args) -> Result<(), CliError> {
    let config = read_local_config(true, args)?;
    if std::env::var(CONFIRM_ENV).ok().as_deref() != Some(CONFIRM_VALUE) {
        return Err(CliError::ConfirmationRequired);
    }
    let reader = build_live_reader(&config.app_key_file, &config.app_secret_file)?;
    let outcome = capture_raw(
        &RawStore::new(&config.raw_root),
        &reader,
        &config.universe,
        &config.identity,
        domain::UtcTimestamp::now(),
    )
    .await
    .map_err(CliError::Capture)?;
    print_success(args, &outcome);
    Ok(())
}

fn read_local_config(execute: bool, args: Args) -> Result<LocalConfig, CliError> {
    let raw_root = required_env(RAW_ROOT_ENV)?;
    let entitlement_reference = required_env(ENTITLEMENT_REFERENCE_ENV)?;
    let entitlement_hash = parse_entitlement_hash(&required_env(ENTITLEMENT_HASH_ENV)?)?;
    let capture_commit = required_env(COMMIT_ENV)?;
    let app_key_file = required_path_env(APP_KEY_FILE_ENV)?;
    let app_secret_file = required_path_env(APP_SECRET_FILE_ENV)?;
    if !Path::new(&raw_root).is_absolute()
        || !Path::new(&app_key_file).is_absolute()
        || !Path::new(&app_secret_file).is_absolute()
        || !Path::new(&app_key_file).is_file()
        || !Path::new(&app_secret_file).is_file()
    {
        return Err(CliError::InvalidConfiguration);
    }
    let entitlement = load_entitlement(&default_entitlement_path())
        .map_err(|_| CliError::InvalidConfiguration)?;
    validate_entitlement_binding(&entitlement_reference, &entitlement_hash, &entitlement)
        .map_err(|_| CliError::InvalidConfiguration)?;
    if execute && !Path::new(&raw_root).exists() {
        let Some(parent) = Path::new(&raw_root).parent() else {
            return Err(CliError::InvalidConfiguration);
        };
        if !parent.is_dir() {
            return Err(CliError::InvalidConfiguration);
        }
    }
    let universe_path = default_universe_path();
    let universe = load_universe(&universe_path).map_err(|_| CliError::InvalidConfiguration)?;
    let identity = CaptureIdentity::new(
        &universe,
        args.start,
        args.end,
        entitlement_reference,
        entitlement_hash,
        capture_commit,
    )
    .map_err(|_| CliError::InvalidConfiguration)?;
    Ok(LocalConfig {
        raw_root: PathBuf::from(raw_root),
        universe,
        identity,
        app_key_file: PathBuf::from(app_key_file),
        app_secret_file: PathBuf::from(app_secret_file),
    })
}

fn default_universe_path() -> PathBuf {
    let image_path = Path::new(IMAGE_UNIVERSE_PATH);
    if image_path.is_file() {
        image_path.to_owned()
    } else {
        PathBuf::from(DEFAULT_UNIVERSE_RELATIVE_PATH)
    }
}

fn default_entitlement_path() -> PathBuf {
    let image_path = Path::new(IMAGE_ENTITLEMENT_PATH);
    if image_path.is_file() {
        image_path.to_owned()
    } else {
        PathBuf::from(DEFAULT_ENTITLEMENT_RELATIVE_PATH)
    }
}

fn parse_entitlement_hash(value: &str) -> Result<ContentHash, CliError> {
    let normalized = if value.starts_with("sha256:") {
        value.to_owned()
    } else if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        format!("sha256:{value}")
    } else {
        return Err(CliError::InvalidConfiguration);
    };
    ContentHash::parse(&normalized).map_err(|_| CliError::InvalidConfiguration)
}

fn required_env(name: &str) -> Result<String, CliError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CliError::MissingConfiguration)
}

fn required_path_env(name: &str) -> Result<String, CliError> {
    required_env(name)
}

fn build_live_reader(
    app_key_file: &Path,
    app_secret_file: &Path,
) -> Result<LiveKisClient, CliError> {
    let app_key_ref = CredentialRef::file(app_key_file.to_string_lossy().into_owned());
    let app_secret_ref = CredentialRef::file(app_secret_file.to_string_lossy().into_owned());
    let token_transport =
        LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| CliError::TransportConfiguration)?;
    let read_transport =
        LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| CliError::TransportConfiguration)?;
    let clock = Arc::new(SystemClock);
    let issuer = KisTokenIssuer::new(
        token_transport,
        SystemCredentialSource,
        app_key_ref.clone(),
        app_secret_ref.clone(),
        kis_system_now_ms,
    );
    let tokens = Arc::new(TokenManager::new(clock.clone(), Arc::new(issuer)));
    let limiter = RateLimiter::new(clock, Quota::new(1, 1)).with_quota(
        BucketKey::new(DAILY_BARS_PATH, DAILY_BARS_TR_ID),
        Quota::new(1, 1),
    );
    Ok(KisMarketDataClient::new(
        read_transport,
        TokioSleeper,
        tokens,
        Arc::new(limiter),
        SystemCredentialSource,
        app_key_ref,
        app_secret_ref,
    ))
}

fn kis_system_now_ms() -> i64 {
    SystemClock.now_ms()
}

fn print_success(args: Args, outcome: &CaptureOutcome) {
    print_json(&SuccessRecord {
        status: "success",
        provider: RAW_PROVIDER,
        market: RAW_MARKET,
        contract_version: RAW_CONTRACT_VERSION,
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        batch_id: outcome.batch_id.to_string(),
        planned_gets_before_retries: outcome.planned_gets,
        actual_gets_before_retries: outcome.actual_gets,
        file_count: outcome.entry.files.len(),
        raw_only: true,
        normalized: false,
        publication: false,
    });
}

fn report_parse_error(error: CliError) -> ExitCode {
    eprintln!("kis-stock-price-beta-raw: {}", error_code(&error));
    eprintln!("use --help for usage");
    ExitCode::from(64)
}

fn report_error(args: Args, error: CliError) -> ExitCode {
    print_json(&FailureRecord {
        status: "failed",
        error_code: error_code(&error),
        raw_visibility: "no_complete_batch_claim",
        network_calls: if matches!(args.action, Action::Execute) {
            "possibly_attempted"
        } else {
            "none"
        },
    });
    ExitCode::from(1)
}

fn error_code(error: &CliError) -> &'static str {
    match error {
        CliError::Usage => "CLI_USAGE",
        CliError::InvalidDate => "CLI_DATE_INVALID",
        CliError::InvalidRange => "CLI_RANGE_INVALID",
        CliError::FixedRangeRequired => "CLI_FIXED_RANGE_REQUIRED",
        CliError::ConflictingAction => "CLI_ACTION_CONFLICT",
        CliError::MissingConfiguration => "CONFIG_MISSING",
        CliError::InvalidConfiguration => "CONFIG_INVALID",
        CliError::ConfirmationRequired => "OPERATOR_ACK_REQUIRED",
        CliError::TransportConfiguration => "KIS_TRANSPORT_CONFIG_INVALID",
        CliError::Capture(error) => error.code(),
    }
}

fn print_json<T: Serialize>(record: &T) {
    if let Ok(json) = serde_json::to_string(record) {
        println!("{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Args {
        let values = values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        parse_args(&values).expect("parse").expect("args")
    }

    #[test]
    fn no_action_uses_the_fixed_default_range_and_plan() {
        let args = parse(&[]);
        assert_eq!(args.action, Action::Plan);
        assert_eq!(args.start.to_iso(), DEFAULT_START);
        assert_eq!(args.end.to_iso(), DEFAULT_END);
    }

    #[test]
    fn conflicting_actions_and_reversed_ranges_fail_closed() {
        let values = ["--plan", "--check"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(matches!(
            parse_args(&values),
            Err(CliError::ConflictingAction)
        ));
        let values = ["--start", "2026-08-28", "--end", "2025-08-04"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(matches!(parse_args(&values), Err(CliError::InvalidRange)));

        let values = ["--start", "2026-01-02", "--end", "2026-01-30"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(matches!(
            parse_args(&values),
            Err(CliError::FixedRangeRequired)
        ));
    }

    #[test]
    fn entitlement_hash_accepts_canonical_and_raw_hex_forms() {
        let canonical = parse_entitlement_hash(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("canonical");
        let raw = parse_entitlement_hash(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("raw");
        assert_eq!(canonical, raw);
        assert!(parse_entitlement_hash("not-a-hash").is_err());
    }
}
