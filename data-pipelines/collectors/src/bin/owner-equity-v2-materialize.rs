use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use collectors::owner_equity_v2::materialize_owner_equity_from_raw;
use domain::CodeCommit;
use market_data::RawStore;
use market_data::owner_equity_v2::OwnerEquityCaptureIdentity;

fn main() -> ExitCode {
    match run() {
        Ok(hash) => {
            println!("{hash}");
            ExitCode::SUCCESS
        }
        Err(code) => {
            eprintln!("{code}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, &'static str> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 4 {
        return Err("USAGE_INVALID");
    }
    let identity_bytes = fs::read(&args[1]).map_err(|_| "IDENTITY_READ_FAILED")?;
    let identity: OwnerEquityCaptureIdentity =
        serde_json::from_slice(&identity_bytes).map_err(|_| "IDENTITY_INVALID")?;
    identity.validate().map_err(|error| error.code())?;
    let commit = CodeCommit::parse(&args[2]).map_err(|_| "COMMIT_INVALID")?;
    let outcome = materialize_owner_equity_from_raw(&RawStore::new(&args[0]), &identity, commit)
        .map_err(|error| error.code())?;
    write_immutable(Path::new(&args[3]), &outcome.canonical_bytes)?;
    Ok(outcome.content_sha256.to_string())
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(bytes)
                .map_err(|_| "CANDIDATE_WRITE_FAILED")?;
            file.sync_all().map_err(|_| "CANDIDATE_WRITE_FAILED")?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::read(path).map_err(|_| "CANDIDATE_READ_FAILED")?;
            if existing == bytes {
                Ok(())
            } else {
                Err("CANDIDATE_IMMUTABLE_CONFLICT")
            }
        }
        Err(_) => Err("CANDIDATE_WRITE_FAILED"),
    }
}
