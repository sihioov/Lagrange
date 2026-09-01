use std::process::ExitCode;

use domain::{OwnerEquityUniversePolicy, TradingDate};
use market_data::owner_equity_v2::{OwnerEquityCaptureKind, OwnerEquityCapturePlan};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("{code}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), &'static str> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 6 {
        return Err("USAGE_INVALID");
    }
    let kind = match args[3].as_str() {
        "initial" => OwnerEquityCaptureKind::Initial,
        "incremental" => OwnerEquityCaptureKind::Incremental,
        _ => return Err("CAPTURE_KIND_INVALID"),
    };
    let target = args[4].parse::<u32>().map_err(|_| "POLICY_INVALID")?;
    let minimum = args[5].parse::<u32>().map_err(|_| "POLICY_INVALID")?;
    let policy =
        OwnerEquityUniversePolicy::new(100, target, minimum).map_err(|_| "POLICY_INVALID")?;
    let plan = OwnerEquityCapturePlan::build(
        &args[0],
        policy,
        kind,
        TradingDate::parse(&args[1]).map_err(|_| "RANGE_INVALID")?,
        TradingDate::parse(&args[2]).map_err(|_| "RANGE_INVALID")?,
    )
    .map_err(|error| error.code())?;
    println!(
        "{}",
        serde_json::to_string(&plan).map_err(|_| "CANONICALIZATION_FAILED")?
    );
    Ok(())
}
