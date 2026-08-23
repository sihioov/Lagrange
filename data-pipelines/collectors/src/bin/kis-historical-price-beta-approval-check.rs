//! Read-only, independently configured approval check for a sealed historical
//! price-only beta artifact.
//!
//! This binary never materializes an artifact, contacts Raw, Curated, a
//! provider, or a database, and has no registry-writing operation.  Its sole
//! authority is to compare safe facts returned by the descriptor-safe artifact
//! reader with a separately committed canonical approval registry.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use domain::{ContentHash, TradingDate};
use market_data::{
    HistoricalPriceOnlyArtifactApprovalSummary, KR_ETF_CORE_SYMBOLS,
    read_historical_price_only_artifact,
};
use serde::{Deserialize, Serialize};

const USAGE: &str =
    "kis-historical-price-beta-approval-check check --artifact-root <ABS_ARTIFACT_ROOT>";
const REGISTRY_SCHEMA_ID: &str = "kis-historical-price-only-beta-approval-registry";
const REGISTRY_SCHEMA_VERSION: u32 = 1;
const MAX_REGISTRY_BYTES: u64 = 256 * 1024;
const EMBEDDED_APPROVAL_REGISTRY_BYTES: &[u8] = include_bytes!(
    "../../../../configs/evidence/kis-historical-price-only-beta-approved-artifacts.json"
);

const CHECK_OPTIONS: [&str; 1] = ["--artifact-root"];

struct CheckArgs {
    artifact_root: PathBuf,
}

enum ParseOutcome {
    Help,
    Check(CheckArgs),
}

#[derive(Debug, Clone, Copy)]
struct Failure {
    reason: &'static str,
}

impl Failure {
    const fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ApprovalRegistry {
    schema_id: String,
    schema_version: u32,
    approved_artifacts: Vec<ApprovedArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ApprovedArtifact {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    artifact_schema_id: String,
    artifact_schema_version: u32,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    bar_count: usize,
}

#[derive(Clone)]
struct ApprovalFacts {
    candidate_content_sha256: ContentHash,
    artifact_manifest_sha256: ContentHash,
    stage5_manifest_sha256: ContentHash,
    action_manifest_sha256: ContentHash,
    approval_registry_sha256: ContentHash,
    artifact_schema_id: String,
    artifact_schema_version: u32,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    bar_count: usize,
}

impl ApprovalFacts {
    fn from_verified(
        candidate_content_sha256: &ContentHash,
        summary: &HistoricalPriceOnlyArtifactApprovalSummary,
        approval_registry_sha256: ContentHash,
    ) -> Self {
        Self {
            candidate_content_sha256: candidate_content_sha256.clone(),
            artifact_manifest_sha256: summary.artifact_manifest_sha256().clone(),
            stage5_manifest_sha256: summary.stage5_manifest_sha256().clone(),
            action_manifest_sha256: summary.action_manifest_sha256().clone(),
            approval_registry_sha256,
            artifact_schema_id: summary.schema_id().to_owned(),
            artifact_schema_version: summary.schema_version(),
            audience: summary.audience().to_owned(),
            vendor_snapshot: summary.vendor_snapshot(),
            strict_pit: summary.strict_pit(),
            capability: summary.capability().to_owned(),
            materialization_status: summary.materialization_status().to_owned(),
            registration_status: summary.registration_status().to_owned(),
            publication_status: summary.publication_status().to_owned(),
            range_start: summary.range_start(),
            range_end: summary.range_end(),
            instruments: summary.instruments().to_vec(),
            instrument_count: summary.instrument_count(),
            session_count: summary.session_count(),
            bar_count: summary.bar_count(),
        }
    }
}

fn main() -> ExitCode {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    match parse_args(&argv) {
        Ok(ParseOutcome::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Check(args)) => match execute_check(&args) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(failure) => {
                eprintln!("{}", failure_line(failure));
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            eprintln!("{}", failure_line(failure));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(argv: &[String]) -> Result<ParseOutcome, Failure> {
    if argv.len() == 1 && argv[0] == "--help" {
        return Ok(ParseOutcome::Help);
    }
    if argv.first().map(String::as_str) != Some("check") {
        return Err(Failure::new(if argv.is_empty() {
            "missing_command"
        } else {
            "unknown_command"
        }));
    }
    let values = exact_option_values(&argv[1..])?;
    let artifact_root = absolute_path(values[0], "artifact_root_must_be_absolute")?;
    Ok(ParseOutcome::Check(CheckArgs { artifact_root }))
}

fn exact_option_values(argv: &[String]) -> Result<Vec<&str>, Failure> {
    if argv.len() != CHECK_OPTIONS.len() * 2 {
        return Err(Failure::new(
            if argv.last().is_some_and(|value| value.starts_with("--")) {
                "missing_option_value"
            } else if argv.len() < CHECK_OPTIONS.len() * 2 {
                "missing_option"
            } else {
                "unknown_or_repeated_option"
            },
        ));
    }
    let mut values = Vec::with_capacity(CHECK_OPTIONS.len());
    for (index, expected) in CHECK_OPTIONS.iter().enumerate() {
        if argv[index * 2] != *expected {
            return Err(Failure::new("unknown_or_repeated_option"));
        }
        values.push(argv[index * 2 + 1].as_str());
    }
    Ok(values)
}

fn absolute_path(value: &str, reason: &'static str) -> Result<PathBuf, Failure> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(Failure::new(reason))
    }
}

fn execute_check(args: &CheckArgs) -> Result<String, Failure> {
    execute_check_with_registry(args, EMBEDDED_APPROVAL_REGISTRY_BYTES)
}

fn execute_check_with_registry(args: &CheckArgs, registry_bytes: &[u8]) -> Result<String, Failure> {
    let registry = read_registry_bytes(registry_bytes)?;
    let approved = sole_approved_artifact(&registry)?;
    let verified = read_historical_price_only_artifact(
        &args.artifact_root,
        &approved.candidate_content_sha256,
    )
    .map_err(|_| Failure::new("artifact_rejected"))?;
    let facts = ApprovalFacts::from_verified(
        verified.candidate_content_sha256(),
        verified.approval_summary(),
        ContentHash::from_bytes(registry_bytes),
    );
    if !fixed_contract_matches(&facts) || !approved_record_matches(approved, &facts) {
        return Err(Failure::new("artifact_not_approved"));
    }
    Ok(success_line(&facts))
}

fn read_registry_bytes(bytes: &[u8]) -> Result<ApprovalRegistry, Failure> {
    if bytes.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(Failure::new("registry_invalid"));
    }
    let registry: ApprovalRegistry =
        serde_json::from_slice(bytes).map_err(|_| Failure::new("registry_invalid"))?;
    let mut canonical =
        serde_json::to_vec(&registry).map_err(|_| Failure::new("registry_invalid"))?;
    canonical.push(b'\n');
    if bytes != canonical || !valid_registry(&registry) {
        return Err(Failure::new("registry_invalid"));
    }
    Ok(registry)
}

fn valid_registry(registry: &ApprovalRegistry) -> bool {
    registry.schema_id == REGISTRY_SCHEMA_ID && registry.schema_version == REGISTRY_SCHEMA_VERSION
}

fn fixed_contract_matches(facts: &ApprovalFacts) -> bool {
    let mut expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    expected_instruments.sort();
    facts.artifact_schema_id == "kis-historical-price-only-beta"
        && facts.artifact_schema_version == 1
        && facts.audience == "OWNER_ONLY"
        && facts.vendor_snapshot
        && !facts.strict_pit
        && facts.capability == "PRICE_RETURN_ONLY"
        && facts.materialization_status == "MATERIALIZED"
        && facts.registration_status == "UNREGISTERED"
        && facts.publication_status == "NOT_PUBLISHED"
        && facts.range_start.to_string() == "2020-01-31"
        && facts.range_end.to_string() == "2026-08-19"
        && facts.instruments == expected_instruments
        && facts.instrument_count == KR_ETF_CORE_SYMBOLS.len()
        && facts.session_count == 1608
        && facts.bar_count == 17688
}

fn sole_approved_artifact(registry: &ApprovalRegistry) -> Result<&ApprovedArtifact, Failure> {
    match registry.approved_artifacts.as_slice() {
        [] => Err(Failure::new("artifact_not_approved")),
        [approved] => Ok(approved),
        _ => Err(Failure::new("registry_ambiguous")),
    }
}

fn approved_record_matches(record: &ApprovedArtifact, facts: &ApprovalFacts) -> bool {
    record.candidate_content_sha256 == facts.candidate_content_sha256
        && record.artifact_manifest_sha256 == facts.artifact_manifest_sha256
        && record.stage5_manifest_sha256 == facts.stage5_manifest_sha256
        && record.action_manifest_sha256 == facts.action_manifest_sha256
        && record.artifact_schema_id == facts.artifact_schema_id
        && record.artifact_schema_version == facts.artifact_schema_version
        && record.audience == facts.audience
        && record.vendor_snapshot == facts.vendor_snapshot
        && record.strict_pit == facts.strict_pit
        && record.capability == facts.capability
        && record.materialization_status == facts.materialization_status
        && record.registration_status == facts.registration_status
        && record.publication_status == facts.publication_status
        && record.range_start == facts.range_start
        && record.range_end == facts.range_end
        && record.instruments == facts.instruments
        && record.instrument_count == facts.instrument_count
        && record.session_count == facts.session_count
        && record.bar_count == facts.bar_count
}

fn success_line(facts: &ApprovalFacts) -> String {
    format!(
        "HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256={} approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count={} session_count={} bar_count={}",
        facts.approval_registry_sha256,
        facts.instrument_count,
        facts.session_count,
        facts.bar_count,
    )
}

fn failure_line(failure: Failure) -> String {
    format!(
        "HISTORICAL_PRICE_BETA_APPROVAL status=blocked operation=check reason={}",
        failure.reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> ContentHash {
        ContentHash::from_bytes(value.as_bytes())
    }

    fn facts() -> ApprovalFacts {
        let mut instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<Vec<_>>();
        instruments.sort();
        ApprovalFacts {
            candidate_content_sha256: hash("candidate"),
            artifact_manifest_sha256: hash("artifact-manifest"),
            stage5_manifest_sha256: hash("stage5-manifest"),
            action_manifest_sha256: hash("action-manifest"),
            approval_registry_sha256: hash("approval-registry"),
            artifact_schema_id: "kis-historical-price-only-beta".into(),
            artifact_schema_version: 1,
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            capability: "PRICE_RETURN_ONLY".into(),
            materialization_status: "MATERIALIZED".into(),
            registration_status: "UNREGISTERED".into(),
            publication_status: "NOT_PUBLISHED".into(),
            range_start: TradingDate::parse("2020-01-31").unwrap(),
            range_end: TradingDate::parse("2026-08-19").unwrap(),
            instruments,
            instrument_count: KR_ETF_CORE_SYMBOLS.len(),
            session_count: 1608,
            bar_count: 17688,
        }
    }

    fn approved(facts: &ApprovalFacts) -> ApprovedArtifact {
        ApprovedArtifact {
            candidate_content_sha256: facts.candidate_content_sha256.clone(),
            artifact_manifest_sha256: facts.artifact_manifest_sha256.clone(),
            stage5_manifest_sha256: facts.stage5_manifest_sha256.clone(),
            action_manifest_sha256: facts.action_manifest_sha256.clone(),
            artifact_schema_id: facts.artifact_schema_id.clone(),
            artifact_schema_version: facts.artifact_schema_version,
            audience: facts.audience.clone(),
            vendor_snapshot: facts.vendor_snapshot,
            strict_pit: facts.strict_pit,
            capability: facts.capability.clone(),
            materialization_status: facts.materialization_status.clone(),
            registration_status: facts.registration_status.clone(),
            publication_status: facts.publication_status.clone(),
            range_start: facts.range_start,
            range_end: facts.range_end,
            instruments: facts.instruments.clone(),
            instrument_count: facts.instrument_count,
            session_count: facts.session_count,
            bar_count: facts.bar_count,
        }
    }

    fn registry(records: Vec<ApprovedArtifact>) -> ApprovalRegistry {
        ApprovalRegistry {
            schema_id: REGISTRY_SCHEMA_ID.into(),
            schema_version: REGISTRY_SCHEMA_VERSION,
            approved_artifacts: records,
        }
    }

    fn canonical_registry_bytes(registry: &ApprovalRegistry) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(registry).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn canonical_registry_approves_only_the_exact_safe_artifact_facts() {
        let facts = facts();
        let registry_bytes = canonical_registry_bytes(&registry(vec![approved(&facts)]));
        let registry = read_registry_bytes(&registry_bytes).unwrap();
        assert!(valid_registry(&registry));
        assert!(fixed_contract_matches(&facts));
        assert!(approved_record_matches(
            sole_approved_artifact(&registry).unwrap(),
            &facts
        ));
        let line = success_line(&facts);
        assert_eq!(
            line,
            format!(
                "HISTORICAL_PRICE_BETA_APPROVAL status=ok operation=check approval_registry_sha256={} approval_status=APPROVED audience=OWNER_ONLY vendor_snapshot=true strict_pit=false capability=PRICE_RETURN_ONLY materialization_status=MATERIALIZED registration_status=UNREGISTERED publication_status=NOT_PUBLISHED instrument_count=11 session_count=1608 bar_count=17688",
                facts.approval_registry_sha256
            )
        );
        for private_fact in [
            facts.candidate_content_sha256.to_string(),
            facts.artifact_manifest_sha256.to_string(),
            facts.stage5_manifest_sha256.to_string(),
            facts.action_manifest_sha256.to_string(),
        ] {
            assert!(!line.contains(&private_fact));
        }
    }

    #[test]
    fn approval_registry_pin_binds_the_exact_embedded_bytes() {
        let exact = ContentHash::from_bytes(EMBEDDED_APPROVAL_REGISTRY_BYTES);
        let mut changed = EMBEDDED_APPROVAL_REGISTRY_BYTES.to_vec();
        changed.push(b' ');

        assert_ne!(exact, ContentHash::from_bytes(&changed));

        let mut facts = facts();
        facts.approval_registry_sha256 = exact.clone();
        assert!(success_line(&facts).contains(&format!("approval_registry_sha256={exact}")));
    }

    #[test]
    fn registry_requires_exactly_one_approved_artifact() {
        let facts = facts();
        assert_eq!(
            sole_approved_artifact(&registry(vec![]))
                .unwrap_err()
                .reason,
            "artifact_not_approved"
        );
        assert_eq!(
            sole_approved_artifact(&registry(vec![approved(&facts), approved(&facts)]))
                .unwrap_err()
                .reason,
            "registry_ambiguous"
        );
    }

    #[test]
    fn embedded_empty_registry_never_approves() {
        let registry = read_registry_bytes(EMBEDDED_APPROVAL_REGISTRY_BYTES).unwrap();
        assert!(registry.approved_artifacts.is_empty());
        assert_eq!(
            sole_approved_artifact(&registry).unwrap_err().reason,
            "artifact_not_approved"
        );
    }

    #[test]
    fn registry_rejects_unknown_stale_and_noncanonical_forms() {
        let facts = facts();
        let unknown = String::from_utf8(canonical_registry_bytes(&registry(vec![])))
            .unwrap()
            .replacen("{", "{\"unexpected\":true,", 1);
        assert!(read_registry_bytes(unknown.as_bytes()).is_err());

        let duplicate = registry(vec![approved(&facts), approved(&facts)]);
        let duplicate = read_registry_bytes(&canonical_registry_bytes(&duplicate)).unwrap();
        assert_eq!(
            sole_approved_artifact(&duplicate).unwrap_err().reason,
            "registry_ambiguous"
        );

        let mut stale = registry(vec![]);
        stale.schema_version = 2;
        assert!(read_registry_bytes(&canonical_registry_bytes(&stale)).is_err());

        assert!(read_registry_bytes(b"{\n\"schema_id\":\"kis-historical-price-only-beta-approval-registry\",\"schema_version\":1,\"approved_artifacts\":[]\n}\n").is_err());
    }

    #[test]
    fn hash_flag_and_count_tamper_cannot_match_an_approved_record() {
        let facts = facts();
        let mut wrong_hash = approved(&facts);
        wrong_hash.artifact_manifest_sha256 = hash("tampered");
        assert!(!approved_record_matches(&wrong_hash, &facts));
        let mut wrong_stage5_hash = approved(&facts);
        wrong_stage5_hash.stage5_manifest_sha256 = hash("tampered-stage5");
        assert!(!approved_record_matches(&wrong_stage5_hash, &facts));
        let mut wrong_action_hash = approved(&facts);
        wrong_action_hash.action_manifest_sha256 = hash("tampered-action");
        assert!(!approved_record_matches(&wrong_action_hash, &facts));
        let mut wrong_flag = approved(&facts);
        wrong_flag.strict_pit = true;
        assert!(!approved_record_matches(&wrong_flag, &facts));
        let mut wrong_count = approved(&facts);
        wrong_count.bar_count += 1;
        assert!(!approved_record_matches(&wrong_count, &facts));
    }

    #[test]
    fn fixed_contract_blocks_matching_but_unsafe_record_facts() {
        let facts = facts();
        for mutate in [
            |facts: &mut ApprovalFacts| facts.strict_pit = true,
            |facts: &mut ApprovalFacts| facts.artifact_schema_version = 2,
            |facts: &mut ApprovalFacts| facts.range_end = TradingDate::parse("2026-08-18").unwrap(),
            |facts: &mut ApprovalFacts| facts.instruments.reverse(),
            |facts: &mut ApprovalFacts| facts.bar_count += 1,
        ] {
            let mut tampered = facts.clone();
            mutate(&mut tampered);
            assert!(approved_record_matches(&approved(&tampered), &tampered));
            assert!(!fixed_contract_matches(&tampered));
        }
    }

    #[test]
    fn grammar_is_exact_and_reader_failure_does_not_write() {
        let root = tempfile::tempdir().unwrap();
        let argv = vec![
            "check".into(),
            "--artifact-root".into(),
            root.path().display().to_string(),
        ];
        let ParseOutcome::Check(parsed) = parse_args(&argv).unwrap() else {
            panic!("check command should parse");
        };
        assert!(execute_check(&parsed).is_err());
        assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());

        let reordered = vec![
            "check".into(),
            "--candidate-content-sha256".into(),
            hash("candidate").to_string(),
            "--artifact-root".into(),
            root.path().display().to_string(),
        ];
        assert!(parse_args(&reordered).is_err());

        let mut caller_supplied_registry = argv;
        caller_supplied_registry.extend([
            "--approval-registry".into(),
            "/tmp/self-approved.json".into(),
        ]);
        assert!(parse_args(&caller_supplied_registry).is_err());
    }

    #[test]
    fn registry_sentinel_never_reaches_the_static_failure_line() {
        let sentinel = "registry-sentinel-must-not-leak";
        let failure =
            read_registry_bytes(format!("{{\"bad\":\"{sentinel}\"}}\n").as_bytes()).unwrap_err();
        assert_eq!(failure.reason, "registry_invalid");
        assert!(!failure_line(failure).contains(sentinel));
    }
}
