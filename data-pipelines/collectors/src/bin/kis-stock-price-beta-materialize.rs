//! Provider-free materialize/check/proposal command for a pre-existing Raw batch.
use collectors::stock_price_beta_materialize::{MaterializeRequest, check, materialize, proposal};
use domain::BatchId;
use std::{env, fs, io::Write, path::PathBuf, process::ExitCode};
const USAGE: &str = "kis-stock-price-beta-materialize <materialize|check|proposal> --raw-root <ABS> --artifact-root <ABS> --universe <ABS> --entitlement <ABS> --batch-id <UUID> --capture-commit <40lowerhex> [--registry <ABS>|--output <ABS>] [--confirm I_CONFIRM_PROVIDER_FREE_RAW_MATERIALIZATION]";
fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(line) => {
            println!("STOCK_PRICE_BETA status=ok {line}");
            ExitCode::SUCCESS
        }
        Err(code) => {
            eprintln!("STOCK_PRICE_BETA status=blocked reason={code}");
            ExitCode::FAILURE
        }
    }
}
fn run(v: Vec<String>) -> Result<&'static str, &'static str> {
    if v.len() == 1 && v[0] == "--help" {
        println!("{USAGE}");
        return Ok("operation=help");
    }
    let op = v.first().map(String::as_str).ok_or("usage")?;
    let get = |name: &str| -> Result<PathBuf, &'static str> {
        let i = v.iter().position(|x| x == name).ok_or("usage")?;
        if i + 1 >= v.len() {
            return Err("usage");
        }
        let p = PathBuf::from(&v[i + 1]);
        if !p.is_absolute() {
            return Err("absolute_path_required");
        }
        Ok(p)
    };
    let raw = get("--raw-root")?;
    let artifact = get("--artifact-root")?;
    let universe = fs::read(get("--universe")?).map_err(|_| "input_read")?;
    let entitlement = fs::read(get("--entitlement")?).map_err(|_| "input_read")?;
    let id = v
        .iter()
        .position(|x| x == "--batch-id")
        .and_then(|i| v.get(i + 1))
        .ok_or("usage")
        .and_then(|x| {
            uuid::Uuid::parse_str(x)
                .map(BatchId::from_uuid)
                .map_err(|_| "batch_id")
        })?;
    let commit = v
        .iter()
        .position(|x| x == "--capture-commit")
        .and_then(|i| v.get(i + 1))
        .ok_or("usage")?
        .clone();
    let request = MaterializeRequest {
        raw_root: raw,
        artifact_root: artifact,
        universe_bytes: universe,
        entitlement_bytes: entitlement,
        batch_id: id,
        capture_commit: commit,
    };
    match op {
        "materialize" => {
            confirmation(&v)?;
            let out = materialize(&request).map_err(|_| "materialize")?;
            if out.artifact.bars.len() != out.artifact.sessions.len() * 30 {
                return Err("count");
            };
            Ok(
                "operation=materialize materialized=MATERIALIZED registration=UNREGISTERED publication=NOT_PUBLISHED",
            )
        }
        "check" => {
            let registry = fs::read(get("--registry")?).map_err(|_| "registry_read")?;
            check(&request, &registry).map_err(|_| "check")?;
            Ok("operation=check approval=verified")
        }
        "proposal" => {
            confirmation(&v)?;
            let out = materialize(&request).map_err(|_| "materialize")?;
            let output = get("--output")?;
            if output.starts_with(env::current_dir().map_err(|_| "output")?) {
                return Err("proposal_output_must_be_outside_git");
            };
            let bytes = serde_json::to_vec(&proposal(&out)).map_err(|_| "proposal")?;
            write_proposal(&output, &bytes)?;
            Ok("operation=proposal materialized=UNREGISTERED publication=NOT_PUBLISHED")
        }
        _ => Err("usage"),
    }
}

fn write_proposal(path: &std::path::Path, bytes: &[u8]) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("output")?;
    if fs::symlink_metadata(parent)
        .map_err(|_| "output")?
        .file_type()
        .is_symlink()
    {
        return Err("output");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut options = fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW);
        let mut file = options.open(path).map_err(|_| "output")?;
        file.write_all(bytes).map_err(|_| "output")?;
        file.sync_all().map_err(|_| "output")?;
        if fs::metadata(path)
            .map_err(|_| "output")?
            .permissions()
            .mode()
            & 0o777
            != 0o600
        {
            return Err("output");
        }
        fs::File::open(parent)
            .map_err(|_| "output")?
            .sync_all()
            .map_err(|_| "output")?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, bytes);
        Err("output")
    }
}

fn confirmation(values: &[String]) -> Result<(), &'static str> {
    let value = values
        .iter()
        .position(|value| value == "--confirm")
        .and_then(|index| values.get(index + 1));
    if value.map(String::as_str) == Some("I_CONFIRM_PROVIDER_FREE_RAW_MATERIALIZATION") {
        Ok(())
    } else {
        Err("confirmation_required")
    }
}
