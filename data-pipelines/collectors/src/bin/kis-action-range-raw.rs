//! Operator-gated, Raw-only KIS KSD action-range collector.
//!
//! The default `etf11` scope makes 7 requests for each of the fixed ETF11
//! short codes (77 initial calls), then follows only the exact KSD `M -> N`
//! continuation contract. `whole-market` preserves the blank `SHT_CD` shape
//! consumed by Stage4B-v0, but it is intentionally opt-in because a long
//! historical range may exceed the ten-page class bound.
//!
//! `--plan` and `--check` do not construct a transport, token manager, or
//! RawStore commit. Only `--execute` accepts the exact acknowledgement and
//! creates the existing credentialed KIS read path. This binary has no DB,
//! account, order, or Compose-live integration.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use domain::{TradingDate, UtcTimestamp};
use kis_client::clock::Clock;
use kis_client::live_transport::LiveTransport;
use kis_client::secret::SystemCredentialSource;
use kis_client::token_issuer::KisTokenIssuer;
use kis_client::{
    BucketKey, CredentialRef, KisMarketDataClient, Quota, RateLimiter, SystemClock, TokenManager,
    TokioSleeper,
};
use market_data::contract::PROVIDER_KIS;
use market_data::ingest::{IngestError, ingest_kis_action_range};
use market_data::storage::RawStore;
use market_data::{KIS_ACTION_MAX_PAGES, KR_ETF_CORE_SYMBOLS, KisActionRangeScope, KisProvider};
use serde::Serialize;

const USAGE: &str = "\
kis-action-range-raw --start YYYY-MM-DD --end YYYY-MM-DD [options]\n\
\n\
Options:\n\
  --scope etf11|whole-market  default: etf11\n\
  --plan                     print the local request plan (default)\n\
  --check                    validate local paths only; no network or Raw write\n\
  --execute                  perform the acknowledged credentialed Raw capture\n\
  --help\n\
\n\
Execute-only environment:\n\
  RESEARCH_RAW_ROOT\n\
  RESEARCH_ENTITLEMENT_REFERENCE\n\
  KIS_APP_KEY_FILE\n\
  KIS_APP_SECRET_FILE\n\
  KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS\n\
";

const RAW_ROOT_ENV: &str = "RESEARCH_RAW_ROOT";
const ENTITLEMENT_ENV: &str = "RESEARCH_ENTITLEMENT_REFERENCE";
const APP_KEY_FILE_ENV: &str = "KIS_APP_KEY_FILE";
const APP_SECRET_FILE_ENV: &str = "KIS_APP_SECRET_FILE";
const CONFIRM_ENV: &str = "KIS_ACTION_RANGE_CONFIRM";
const CONFIRM_VALUE: &str = "I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS";
const KIS_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_CHANNELS: [(&str, &str); 6] = [
    (
        "/uapi/domestic-stock/v1/ksdinfo/paidin-capin",
        "HHKDB669100C0",
    ),
    (
        "/uapi/domestic-stock/v1/ksdinfo/bonus-issue",
        "HHKDB669101C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/dividend", "HHKDB669102C0"),
    (
        "/uapi/domestic-stock/v1/ksdinfo/merger-split",
        "HHKDB669104C0",
    ),
    ("/uapi/domestic-stock/v1/ksdinfo/rev-split", "HHKDB669105C0"),
    ("/uapi/domestic-stock/v1/ksdinfo/cap-dcrs", "HHKDB669106C0"),
];

type LiveKisClient = KisMarketDataClient<LiveTransport, TokioSleeper, SystemCredentialSource>;

fn kis_system_now_ms() -> i64 {
    SystemClock.now_ms()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Check,
    Execute,
}

#[derive(Debug, Clone, Copy)]
struct Args {
    start: TradingDate,
    end: TradingDate,
    scope: KisActionRangeScope,
    action: Action,
}

#[derive(Debug)]
struct LocalConfig {
    raw_root: PathBuf,
    entitlement_reference: String,
    app_key_file: PathBuf,
    app_secret_file: PathBuf,
}

#[derive(Debug)]
enum CliError {
    Usage,
    InvalidDate,
    InvalidRange,
    InvalidScope,
    ConflictingAction,
    MissingLocalConfig,
    InvalidLocalConfig,
    ConfirmationRequired,
    LocalTransportUnavailable,
    Capture(IngestError),
}

#[derive(Serialize)]
struct PlanRecord {
    status: &'static str,
    scope: &'static str,
    start: String,
    end: String,
    initial_calls: usize,
    max_pages_per_class: usize,
    fixed_etf11_symbols: usize,
    network_calls: &'static str,
    raw_commit: &'static str,
    stage4b_v0_direct_input: bool,
    bridge_v1_lineage: bool,
    strict_pit: bool,
}

#[derive(Serialize)]
struct CheckRecord {
    status: &'static str,
    scope: &'static str,
    start: String,
    end: String,
    initial_calls: usize,
    local_only: bool,
    credential_paths_checked: bool,
    network_calls: &'static str,
    raw_write: &'static str,
}

#[derive(Serialize)]
struct SuccessRecord {
    status: &'static str,
    provider: &'static str,
    scope: &'static str,
    start: String,
    end: String,
    initial_calls: usize,
    pages: usize,
    raw_capture_complete: bool,
    coverage_claim: &'static str,
    vendor_snapshot: bool,
    strict_pit: bool,
    stage4b_v0_direct_input: bool,
    bridge_v1_lineage: bool,
    batch_id: String,
    file_count: usize,
}

#[derive(Serialize)]
struct FailureRecord {
    status: &'static str,
    provider: &'static str,
    scope: &'static str,
    start: String,
    end: String,
    raw_capture_complete: bool,
    coverage_claim: &'static str,
    error_code: &'static str,
    network_calls: &'static str,
    raw_visibility: &'static str,
}

fn main() -> ExitCode {
    let args = match parse_args(&std::env::args().skip(1).collect::<Vec<_>>()) {
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
        Action::Execute => run_execute(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_error(args, error),
    }
}

fn parse_args(args: &[String]) -> Result<Option<Args>, CliError> {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(None);
    }
    let mut start = None;
    let mut end = None;
    let mut scope = KisActionRangeScope::FixedEtf11;
    let mut action = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--start" => {
                index += 1;
                start = Some(parse_date(args.get(index))?);
            }
            "--end" => {
                index += 1;
                end = Some(parse_date(args.get(index))?);
            }
            "--scope" => {
                index += 1;
                scope = match args.get(index).map(String::as_str) {
                    Some("etf11") => KisActionRangeScope::FixedEtf11,
                    Some("whole-market") => KisActionRangeScope::WholeMarket,
                    _ => return Err(CliError::InvalidScope),
                };
            }
            "--plan" => set_action(&mut action, Action::Plan)?,
            "--check" => set_action(&mut action, Action::Check)?,
            "--execute" => set_action(&mut action, Action::Execute)?,
            _ => return Err(CliError::Usage),
        }
        index += 1;
    }
    let (Some(start), Some(end)) = (start, end) else {
        return Err(CliError::Usage);
    };
    if end < start {
        return Err(CliError::InvalidRange);
    }
    Ok(Some(Args {
        start,
        end,
        scope,
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
    let record = PlanRecord {
        status: "plan",
        scope: args.scope.as_str(),
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        initial_calls: args.scope.initial_call_count(),
        max_pages_per_class: KIS_ACTION_MAX_PAGES,
        fixed_etf11_symbols: KR_ETF_CORE_SYMBOLS.len(),
        network_calls: "none",
        raw_commit: "none",
        stage4b_v0_direct_input: matches!(args.scope, KisActionRangeScope::WholeMarket),
        bridge_v1_lineage: true,
        strict_pit: false,
    };
    print_json(&record);
}

fn run_check(args: Args) -> Result<(), CliError> {
    let _config = read_local_config(false)?;
    let record = CheckRecord {
        status: "check",
        scope: args.scope.as_str(),
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        initial_calls: args.scope.initial_call_count(),
        local_only: true,
        credential_paths_checked: true,
        network_calls: "none",
        raw_write: "none",
    };
    print_json(&record);
    Ok(())
}

fn run_execute(args: Args) -> Result<(), CliError> {
    let config = read_local_config(true)?;
    if std::env::var(CONFIRM_ENV).ok().as_deref() != Some(CONFIRM_VALUE) {
        return Err(CliError::ConfirmationRequired);
    }
    let provider = build_provider(&config.app_key_file, &config.app_secret_file)?;
    let store = RawStore::new(&config.raw_root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| CliError::LocalTransportUnavailable)?;
    let outcome = runtime
        .block_on(ingest_kis_action_range(
            &store,
            &provider,
            "kr",
            args.start,
            args.end,
            UtcTimestamp::now(),
            args.scope,
            Some(&config.entitlement_reference),
        ))
        .map_err(CliError::Capture)?;
    let pages = outcome
        .entry
        .files
        .len()
        .saturating_sub(args.scope.initial_call_count());
    let record = SuccessRecord {
        status: "success",
        provider: PROVIDER_KIS,
        scope: args.scope.as_str(),
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        initial_calls: args.scope.initial_call_count(),
        pages,
        raw_capture_complete: true,
        coverage_claim: "not_asserted",
        vendor_snapshot: true,
        strict_pit: false,
        stage4b_v0_direct_input: matches!(args.scope, KisActionRangeScope::WholeMarket)
            && outcome.entry.files.len() == args.scope.initial_call_count(),
        bridge_v1_lineage: true,
        batch_id: outcome.batch_id.to_string(),
        file_count: outcome.entry.files.len(),
    };
    print_json(&record);
    Ok(())
}

fn read_local_config(execute: bool) -> Result<LocalConfig, CliError> {
    let raw_root = required_env(RAW_ROOT_ENV)?;
    let entitlement_reference = required_env(ENTITLEMENT_ENV)?;
    let app_key_file = required_path_env(APP_KEY_FILE_ENV)?;
    let app_secret_file = required_path_env(APP_SECRET_FILE_ENV)?;
    if !Path::new(&raw_root).is_absolute()
        || !Path::new(&app_key_file).is_absolute()
        || !Path::new(&app_secret_file).is_absolute()
        || entitlement_reference.is_empty()
    {
        return Err(CliError::InvalidLocalConfig);
    }
    if !Path::new(&app_key_file).is_file() || !Path::new(&app_secret_file).is_file() {
        return Err(CliError::InvalidLocalConfig);
    }
    if execute && !Path::new(&raw_root).exists() {
        let Some(parent) = Path::new(&raw_root).parent() else {
            return Err(CliError::InvalidLocalConfig);
        };
        if !parent.is_dir() {
            return Err(CliError::InvalidLocalConfig);
        }
    }
    Ok(LocalConfig {
        raw_root: PathBuf::from(raw_root),
        entitlement_reference,
        app_key_file: PathBuf::from(app_key_file),
        app_secret_file: PathBuf::from(app_secret_file),
    })
}

fn required_env(name: &str) -> Result<String, CliError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CliError::MissingLocalConfig)
}

fn required_path_env(name: &str) -> Result<String, CliError> {
    required_env(name)
}

fn build_provider(
    app_key_file: &Path,
    app_secret_file: &Path,
) -> Result<KisProvider<LiveKisClient>, CliError> {
    let app_key_ref = CredentialRef::file(app_key_file.to_string_lossy().into_owned());
    let app_secret_ref = CredentialRef::file(app_secret_file.to_string_lossy().into_owned());
    let token_transport =
        LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| CliError::LocalTransportUnavailable)?;
    let read_transport =
        LiveTransport::live(KIS_HTTP_TIMEOUT).map_err(|_| CliError::LocalTransportUnavailable)?;
    let clock = Arc::new(SystemClock);
    let token_issuer = KisTokenIssuer::new(
        token_transport,
        SystemCredentialSource,
        app_key_ref.clone(),
        app_secret_ref.clone(),
        kis_system_now_ms,
    );
    let tokens = Arc::new(TokenManager::new(clock.clone(), Arc::new(token_issuer)));
    let limiter = ACTION_CHANNELS.iter().fold(
        RateLimiter::new(clock, Quota::new(1, 1)),
        |limiter, (endpoint, tr_id)| {
            limiter.with_quota(BucketKey::new(*endpoint, *tr_id), Quota::new(1, 1))
        },
    );
    let client = KisMarketDataClient::new(
        read_transport,
        TokioSleeper,
        tokens,
        Arc::new(limiter),
        SystemCredentialSource,
        app_key_ref,
        app_secret_ref,
    );
    Ok(KisProvider::kr_etf_core(client))
}

fn report_parse_error(error: CliError) -> ExitCode {
    eprintln!("kis-action-range-raw: {}", safe_error_code(&error));
    eprintln!("use --help for usage");
    ExitCode::from(64)
}

fn report_error(args: Args, error: CliError) -> ExitCode {
    let record = FailureRecord {
        status: "failed",
        provider: PROVIDER_KIS,
        scope: args.scope.as_str(),
        start: args.start.to_iso(),
        end: args.end.to_iso(),
        raw_capture_complete: false,
        coverage_claim: "not_asserted",
        error_code: safe_error_code(&error),
        network_calls: if matches!(args.action, Action::Execute) {
            "possibly_attempted"
        } else {
            "none"
        },
        raw_visibility: "no_complete_batch_claim",
    };
    print_json(&record);
    ExitCode::from(1)
}

fn safe_error_code(error: &CliError) -> &'static str {
    match error {
        CliError::Usage => "CLI_USAGE",
        CliError::InvalidDate => "CLI_DATE_INVALID",
        CliError::InvalidRange => "CLI_RANGE_INVALID",
        CliError::InvalidScope => "CLI_SCOPE_INVALID",
        CliError::ConflictingAction => "CLI_ACTION_CONFLICT",
        CliError::MissingLocalConfig => "CONFIG_MISSING",
        CliError::InvalidLocalConfig => "CONFIG_INVALID",
        CliError::ConfirmationRequired => "OPERATOR_ACK_REQUIRED",
        CliError::LocalTransportUnavailable => "KIS_TRANSPORT_CONFIG_INVALID",
        CliError::Capture(error) => ingest_error_code(error),
    }
}

fn ingest_error_code(error: &IngestError) -> &'static str {
    match error {
        IngestError::Provider(provider) => match provider {
            market_data::ProviderError::Remote { code, .. } => code,
            market_data::ProviderError::InvalidConfiguration { .. } => "KIS_CONFIG_INVALID",
            market_data::ProviderError::UnsupportedKind(_) => "KIS_KIND_UNSUPPORTED",
            market_data::ProviderError::CredentialsUnavailable { .. } => {
                "KIS_CREDENTIALS_UNAVAILABLE"
            }
            market_data::ProviderError::EndpointTimeout { .. } => "KIS_ENDPOINT_TIMEOUT",
            market_data::ProviderError::UnsafeFileName { .. } => "KIS_FILENAME_INVALID",
            market_data::ProviderError::Io { .. } => "KIS_IO_FAILURE",
            market_data::ProviderError::RecordedBundleMissing { .. }
            | market_data::ProviderError::RecordedBundleIo { .. }
            | market_data::ProviderError::RecordedBundleParse { .. }
            | market_data::ProviderError::RecordedBundleInvalid { .. } => "KIS_PROVIDER_FAILURE",
        },
        IngestError::MalformedResponse {
            diagnostic: Some(diagnostic),
            ..
        } => diagnostic.code,
        IngestError::MalformedResponse { .. } => "KIS_RESPONSE_MALFORMED",
        IngestError::ResponseShape { .. } => "KIS_RESPONSE_SHAPE_INVALID",
        IngestError::Store(_) => "RAW_STORE_FAILURE",
        IngestError::Readback { .. } => "RAW_READBACK_FAILURE",
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

    fn parse(arguments: &[&str]) -> Args {
        let values = arguments
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        parse_args(&values).unwrap().unwrap()
    }

    #[test]
    fn default_is_local_plan_and_etf11_has_77_initial_calls() {
        let args = parse(&["--start", "2020-01-01", "--end", "2026-08-21"]);
        assert_eq!(args.action, Action::Plan);
        assert_eq!(args.scope, KisActionRangeScope::FixedEtf11);
        assert_eq!(args.scope.initial_call_count(), 77);
    }

    #[test]
    fn whole_market_keeps_v0_shape_as_an_explicit_scope() {
        let args = parse(&[
            "--start",
            "2020-01-01",
            "--end",
            "2026-08-21",
            "--scope",
            "whole-market",
        ]);
        assert_eq!(args.scope, KisActionRangeScope::WholeMarket);
        assert_eq!(args.scope.initial_call_count(), 7);
    }

    #[test]
    fn reversed_range_and_conflicting_modes_fail_closed() {
        let values = ["--start", "2026-08-21", "--end", "2020-01-01"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(matches!(parse_args(&values), Err(CliError::InvalidRange)));
        let values = [
            "--start",
            "2020-01-01",
            "--end",
            "2026-08-21",
            "--plan",
            "--execute",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert!(matches!(
            parse_args(&values),
            Err(CliError::ConflictingAction)
        ));
    }
}
