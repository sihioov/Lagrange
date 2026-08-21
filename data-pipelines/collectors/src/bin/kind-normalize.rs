//! First-party, provider-free KIND normalizer CLI.
//!
//! This binary is intentionally a very small adapter around the existing
//! `market-data` normalizers.  It reads an already committed Raw batch, calls
//! the normalizer owned by that crate, and emits only typed counts/identities.
//! It does not launch a browser, use a network client, parse HTML itself, or
//! infer a correction relationship.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use domain::BatchId;
use market_data::{
    KindCorrectionNormalizationOutcome, KindNormalizationOutcome, MARKET_KR, ManifestEntry,
    PROVIDER_KIND_DISCLOSURE, PROVIDER_KIND_DISCLOSURE_CORRECTION, RawStore,
    normalize_kind_correction_batch, normalize_kind_disclosure_batch, parse_kind_disclosure_pages,
};

const MAX_CORRECTION_CANDIDATES: usize = 5;
const MAX_CANDIDATE_FILE_BYTES: usize = 4096;

const USAGE: &str = "kind-normalize --raw-root <DATA_ROOT> --source-batch-id <UUID> \\
    --mode disclosure --candidate-file <FILE>\nkind-normalize --raw-root <DATA_ROOT> \\
    --source-batch-id <UUID> --mode correction\n\nThe raw root is the data/ directory; RawStore places evidence below raw/.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Disclosure,
    Correction,
}

#[derive(Debug)]
struct Args {
    raw_root: PathBuf,
    source_batch_id: BatchId,
    mode: Mode,
    candidate_file: Option<PathBuf>,
}

#[derive(Debug)]
struct Failure(&'static str);

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    if argv.len() == 1 && argv[0] == "--help" {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match parse_args(&argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure(code)) => {
            eprintln!("KIND_NORMALIZE status=incomplete reason={code}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, Failure> {
    let mut raw_root = None;
    let mut source_batch_id = None;
    let mut mode = None;
    let mut candidate_file = None;
    let mut index = 0;

    while index < argv.len() {
        let key = argv[index].as_str();
        let value = argv.get(index + 1).ok_or(Failure("missing_option_value"))?;
        if !key.starts_with("--") {
            return Err(Failure("unexpected_argument"));
        }
        match key {
            "--raw-root" if raw_root.is_none() => raw_root = Some(PathBuf::from(value)),
            "--source-batch-id" if source_batch_id.is_none() => {
                source_batch_id = Some(value.parse().map_err(|_| Failure("invalid_batch_id"))?)
            }
            "--mode" if mode.is_none() => {
                mode = Some(match value.as_str() {
                    "disclosure" => Mode::Disclosure,
                    "correction" => Mode::Correction,
                    _ => return Err(Failure("invalid_mode")),
                })
            }
            "--candidate-file" if candidate_file.is_none() => {
                candidate_file = Some(PathBuf::from(value))
            }
            _ => return Err(Failure("unknown_or_repeated_option")),
        }
        index += 2;
    }

    let raw_root = raw_root.ok_or(Failure("missing_raw_root"))?;
    if !raw_root.is_absolute() {
        return Err(Failure("raw_root_must_be_absolute"));
    }
    let source_batch_id = source_batch_id.ok_or(Failure("missing_source_batch_id"))?;
    let mode = mode.ok_or(Failure("missing_mode"))?;
    match (mode, candidate_file.is_some()) {
        (Mode::Disclosure, false) => Err(Failure("disclosure_candidate_file_required")),
        (Mode::Correction, true) => Err(Failure("correction_candidate_file_forbidden")),
        _ => Ok(Args {
            raw_root,
            source_batch_id,
            mode,
            candidate_file,
        }),
    }
}

fn run(args: Args) -> Result<(), Failure> {
    let raw = RawStore::new(&args.raw_root);
    match args.mode {
        Mode::Disclosure => run_disclosure(
            &raw,
            args.source_batch_id,
            args.candidate_file
                .as_deref()
                .ok_or(Failure("disclosure_candidate_file_required"))?,
        ),
        Mode::Correction => run_correction(&raw, args.source_batch_id),
    }
}

fn find_source(
    raw: &RawStore,
    provider: &'static str,
    source_batch_id: BatchId,
) -> Result<ManifestEntry, Failure> {
    let entries = raw
        .read_manifest(provider, MARKET_KR)
        .map_err(|_| Failure("source_manifest_read_failed"))?;
    let mut matches = entries
        .into_iter()
        .filter(|entry| entry.batch_id == source_batch_id);
    let source = matches.next().ok_or(Failure("source_batch_not_found"))?;
    if matches.next().is_some() {
        return Err(Failure("source_batch_not_unique"));
    }
    Ok(source)
}

fn run_disclosure(
    raw: &RawStore,
    source_batch_id: BatchId,
    candidate_file: &Path,
) -> Result<(), Failure> {
    let source = find_source(raw, PROVIDER_KIND_DISCLOSURE, source_batch_id)?;
    let stored = raw
        .read_batch_bytes(PROVIDER_KIND_DISCLOSURE, MARKET_KR, &source)
        .map_err(|_| Failure("source_batch_read_failed"))?;
    let pages: Vec<(String, Vec<u8>)> = stored
        .iter()
        .map(|file| (file.file_name.clone(), file.bytes.clone()))
        .collect();
    // Use the existing normalizer parser for candidate membership validation;
    // this CLI does not add an HTML parser or a title-marker heuristic.
    let observations = parse_kind_disclosure_pages(&pages)
        .map_err(|_| Failure("disclosure_normalization_validation_failed"))?;
    let candidates = read_candidates(candidate_file)?;
    if candidates.iter().any(|candidate| {
        !observations
            .iter()
            .any(|observation| observation.disclosure_acceptance_number == *candidate)
    }) {
        return Err(Failure("candidate_not_in_normalized_disclosure"));
    }

    let outcome = normalize_kind_disclosure_batch(raw, &source)
        .map_err(|_| Failure("disclosure_normalization_failed"))?;
    print_disclosure_success(&outcome, candidates.len());
    Ok(())
}

fn run_correction(raw: &RawStore, source_batch_id: BatchId) -> Result<(), Failure> {
    let source = find_source(raw, PROVIDER_KIND_DISCLOSURE_CORRECTION, source_batch_id)?;
    // The existing normalizer extracts the opaque anchor from the stored
    // request metadata.  No caller-supplied date or lineage is accepted here.
    let outcome = normalize_kind_correction_batch(raw, &source)
        .map_err(|_| Failure("correction_normalization_failed"))?;
    print_correction_success(&outcome);
    Ok(())
}

fn read_candidates(path: &Path) -> Result<Vec<String>, Failure> {
    let bytes = std::fs::read(path).map_err(|_| Failure("candidate_file_read_failed"))?;
    if bytes.len() > MAX_CANDIDATE_FILE_BYTES {
        return Err(Failure("candidate_file_too_large"));
    }
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(b"\n") {
        return Err(Failure("candidate_file_missing_final_newline"));
    }

    let text = std::str::from_utf8(&bytes).map_err(|_| Failure("candidate_file_not_ascii"))?;
    let mut values = Vec::new();
    for line in text.split_terminator('\n') {
        if !is_ascii_acceptance(line) {
            return Err(Failure("candidate_shape_invalid"));
        }
        values.push(line.to_owned());
    }
    if values.len() > MAX_CORRECTION_CANDIDATES {
        return Err(Failure("correction_candidate_budget_exceeded"));
    }
    let unique: BTreeSet<&str> = values.iter().map(String::as_str).collect();
    if unique.len() != values.len() {
        return Err(Failure("duplicate_correction_candidate"));
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Failure("candidate_order_not_deterministic"));
    }
    Ok(values)
}

fn is_ascii_acceptance(value: &str) -> bool {
    value.len() == 14 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn print_disclosure_success(outcome: &KindNormalizationOutcome, candidate_count: usize) {
    println!(
        "KIND_NORMALIZE mode=disclosure source_batch_id={} normalized_batch_id={} observation_count={} candidate_count={candidate_count}",
        outcome.source_batch_id, outcome.normalized_batch_id, outcome.row_count
    );
}

fn print_correction_success(outcome: &KindCorrectionNormalizationOutcome) {
    println!(
        "KIND_NORMALIZE mode=correction source_batch_id={} normalized_batch_id={} version_count={}",
        outcome.source_batch_id,
        outcome.normalized_batch_id,
        outcome.membership.ordered_versions.len()
    );
}

#[cfg(test)]
mod tests {
    use super::{is_ascii_acceptance, read_candidates};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn candidate_shape_is_opaque_and_exact() {
        assert!(is_ascii_acceptance("20260821000001"));
        assert!(!is_ascii_acceptance("2026082100001"));
        assert!(!is_ascii_acceptance("20260821000001\n"));
        assert!(!is_ascii_acceptance("2026０８２１０００００１"));
    }

    #[test]
    fn candidate_file_is_sorted_unique_and_bounded() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("candidates.txt");
        fs::write(&path, b"20260821000001\n20260821000002\n").expect("write");
        let values = read_candidates(&path).expect("valid candidates");
        assert_eq!(values.len(), 2);

        fs::write(&path, b"20260821000002\n20260821000001\n").expect("write");
        assert!(read_candidates(&path).is_err());
    }
}
