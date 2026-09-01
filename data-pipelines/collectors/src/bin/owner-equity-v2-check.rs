use std::fs;
use std::process::ExitCode;

use collectors::owner_equity_v2::check_owner_equity_from_raw;
use domain::{CodeCommit, ContentHash};
use market_data::RawStore;
use market_data::owner_equity_v2::OwnerEquityCaptureIdentity;

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
    if args.len() != 5 {
        return Err("USAGE_INVALID");
    }
    let identity_bytes = fs::read(&args[1]).map_err(|_| "IDENTITY_READ_FAILED")?;
    let identity: OwnerEquityCaptureIdentity =
        serde_json::from_slice(&identity_bytes).map_err(|_| "IDENTITY_INVALID")?;
    identity.validate().map_err(|error| error.code())?;
    let commit = CodeCommit::parse(&args[2]).map_err(|_| "COMMIT_INVALID")?;
    let candidate_bytes = fs::read(&args[3]).map_err(|_| "CANDIDATE_READ_FAILED")?;
    let expected_hash = ContentHash::parse(&args[4]).map_err(|_| "CANDIDATE_HASH_INVALID")?;
    check_owner_equity_from_raw(
        &RawStore::new(&args[0]),
        &identity,
        commit,
        &candidate_bytes,
        &expected_hash,
    )
    .map_err(|error| error.code())?;
    Ok(())
}
