//! Offline plan/check CLI for the fixed ETF11 identity Raw contract.
//!
//! The historical fixture/provider implementation remains available in the
//! market-data crate for contract tests, but this executable has no live
//! action. Both supported actions are local-only: `--plan` renders the
//! approved query shape and `--check` reads the existing Raw manifest.

use std::fmt;
use std::process::ExitCode;

use domain::TradingDate;
use market_data::storage::StoreError;
use market_data::{
    FSC_KRX_LISTED_ENDPOINT, FSC_KRX_LISTED_PATH, FSC_KRX_LISTED_PROVIDER, ITEM_INFO_MAX_PAGES,
    ITEM_INFO_PAGE_SIZE, MARKET_KR, RawStore,
};

const USAGE: &str = "\
fsc-krx-listed-raw --date <YYYY-MM-DD> (--plan | --check)

Required environment:
  FSC_KRX_LISTED_RAW_ROOT       existing Raw data root

--plan prints the fixed query shape and makes no request.
--check reads the existing Raw manifest and makes no request or write.
";

const RAW_ROOT_ENV_VAR: &str = "FSC_KRX_LISTED_RAW_ROOT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Plan,
    Check,
}

#[derive(Debug, Clone)]
struct ParsedArgs {
    date: Option<TradingDate>,
    action: Action,
}

#[derive(Debug)]
enum CliError {
    Usage(String),
    MissingEnv(&'static str),
    EmptyEnv(&'static str),
    Store(StoreError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(f, "{message}\n\n{USAGE}"),
            Self::MissingEnv(name) => {
                write!(f, "required configuration variable {name} is not set")
            }
            Self::EmptyEnv(name) => write!(f, "required configuration variable {name} is empty"),
            Self::Store(error) => write!(f, "Raw store error: {error}"),
        }
    }
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&raw_args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(raw_args: &[String]) -> Result<(), CliError> {
    let args = parse_args(raw_args)?;
    let raw_root = required_env(RAW_ROOT_ENV_VAR)?;
    let store = RawStore::new(&raw_root);
    match args.action {
        Action::Plan => {
            print!("{}", render_plan(&args, &raw_root));
            Ok(())
        }
        Action::Check => run_check(&store, args.date.as_ref().expect("validated exact date")),
    }
}

fn run_check(store: &RawStore, date: &TradingDate) -> Result<(), CliError> {
    let entries = if store
        .manifest_path(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
        .is_file()
    {
        store
            .read_manifest(FSC_KRX_LISTED_PROVIDER, MARKET_KR)
            .map_err(CliError::Store)?
    } else {
        Vec::new()
    };
    let already_stored = entries.iter().any(|entry| entry.date == *date);
    println!(
        "FSC_KRX_LISTED_CHECK: PASS date={} already_stored={already_stored} network=not-called",
        date.to_iso()
    );
    Ok(())
}

fn render_plan(args: &ParsedArgs, raw_root: &str) -> String {
    format!(
        "provider: {FSC_KRX_LISTED_PROVIDER}\npath: {FSC_KRX_LISTED_PATH}\nendpoint: {FSC_KRX_LISTED_ENDPOINT}\nraw_root: {raw_root}\ntarget_date: {}\nfixed_identity_count: 11\nquery_parameter_names: numOfRows, pageNo, basDt, isinCd, resultType\npage_size: {ITEM_INFO_PAGE_SIZE}\nmax_pages_per_identity: {ITEM_INFO_MAX_PAGES}\nno request was made (--plan)\n",
        args.date.as_ref().expect("plan requires date").to_iso()
    )
}

fn required_env(name: &'static str) -> Result<String, CliError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Err(CliError::EmptyEnv(name)),
        Err(_) => Err(CliError::MissingEnv(name)),
    }
}

fn parse_args(raw_args: &[String]) -> Result<ParsedArgs, CliError> {
    let mut date = None;
    let mut action = None;
    let mut iter = raw_args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--date" => {
                if date.is_some() {
                    return Err(CliError::Usage(
                        "--date may only be specified once".to_owned(),
                    ));
                }
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::Usage("--date requires a value".to_owned()))?;
                date = Some(TradingDate::parse(value).map_err(|_| {
                    CliError::Usage("--date must be a valid YYYY-MM-DD date".to_owned())
                })?);
            }
            "--plan" => set_action(&mut action, Action::Plan)?,
            "--check" => set_action(&mut action, Action::Check)?,
            _ => return Err(CliError::Usage("unrecognized argument".to_owned())),
        }
    }
    let action =
        action.ok_or_else(|| CliError::Usage("exactly one action flag is required".to_owned()))?;
    if date.is_none() {
        return Err(CliError::Usage("--date is required".to_owned()));
    }
    Ok(ParsedArgs { date, action })
}

fn set_action(action: &mut Option<Action>, next: Action) -> Result<(), CliError> {
    if action.replace(next).is_some() {
        return Err(CliError::Usage(
            "action flags are mutually exclusive".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn action_requires_explicit_offline_gate_and_exact_date() {
        assert!(parse_args(&args(&["--date", "2026-08-20"])).is_err());
        let parsed = parse_args(&args(&["--date", "2026-08-20", "--plan"])).unwrap();
        assert_eq!(parsed.action, Action::Plan);
        assert_eq!(parsed.date.unwrap().to_iso(), "2026-08-20");
        assert!(parse_args(&args(&["--date", "2026-08-20", "--execute"])).is_err());
        assert!(parse_args(&args(&["--date", "2026-08-20", "--live"])).is_err());
        assert!(parse_args(&args(&["--date", "2026-08-20", "--probe"])).is_err());
    }

    #[test]
    fn offline_actions_are_mutually_exclusive() {
        assert!(parse_args(&args(&["--date", "2026-08-20", "--plan", "--check"])).is_err());
    }

    #[test]
    fn plan_contains_names_not_isin_or_service_key_values() {
        let parsed = parse_args(&args(&["--date", "2026-08-20", "--plan"])).unwrap();
        let output = render_plan(&parsed, "/synthetic/raw");
        assert!(output.contains("basDt, isinCd, resultType"));
        assert!(!output.contains("KR7069500007"));
        assert!(output.contains("no request was made (--plan)"));
    }
}
