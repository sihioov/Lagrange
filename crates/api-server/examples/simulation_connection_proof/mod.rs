//! Bounded, opt-in production-path proof for the owner-beta simulation API.
//!
//! This driver never starts a worker. It admits one approved short-window job,
//! emits a private readiness receipt, and observes the independently launched
//! runner through the real authenticated router and database contracts.

use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use axum::http::{StatusCode, header};
use http_body_util::BodyExt;
use job_queue::owner_beta::{
    simulation::{EngineIdentity, SimulationResult},
    simulation_execution::{CheckedSimulationPair, SimulationJobInput},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use sqlx::{ConnectOptions, Connection, Row, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::common::{Harness, UserCtx};

const START_DATE: &str = "2017-06-22";
const END_DATE: &str = "2017-06-23";
const DATABASE_URL: &str = "postgres://postgres@localhost/postgres?host=/r4-proof-db";
const ROOT_BASENAME: &str = "historical-price-beta-root";
const PROOF_RECEIPT_NAME: &str = "connection-proof-provenance.json";
const RUNNER_RECEIPT_NAME: &str = "simulation-runner-provenance.json";
const RUNNER_NAME: &str = "owner-beta-simulation-runner";
const READY_NAME: &str = "ready.json";
const REPORT_NAME: &str = "report.json";
const PROOF_SCHEMA: &str = "owner-beta-simulation-connection-proof-build-v1";
const RUNNER_SCHEMA: &str = "owner-beta-simulation-build-v1";
const READY_SCHEMA: &str = "owner-beta-simulation-connection-ready-v1";
const REPORT_SCHEMA: &str = "owner-beta-simulation-connection-proof-v1";
const RUNS_PATH: &str = "/api/v1/backtests/owner-beta/price-only/runs";
const REQUEST_ID: &str = "r4-connection-proof";
const IDEMPOTENCY_KEY: &str = "r4-connection-proof-window-v1";
const DENIAL_IDEMPOTENCY_KEY: &str = "r4-connection-proof-denial-v1";
const MAX_RECEIPT_BYTES: usize = 1024 * 1024;
const MAX_SMALL_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_PAIR_RESPONSE_BYTES: usize = 128 * 1024 * 1024;
const OVERALL_DEADLINE: Duration = Duration::from_secs(1500);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const BODY_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

const BUILD_GIT_COMMIT: Option<&str> = option_env!("R4_CONNECTION_PROOF_BUILD_GIT_COMMIT");
const BUILD_TOOLCHAIN: Option<&str> = option_env!("R4_CONNECTION_PROOF_BUILD_TOOLCHAIN");
const BUILD_TARGET: Option<&str> = option_env!("R4_CONNECTION_PROOF_BUILD_TARGET");
const BUILD_PROFILE: Option<&str> = option_env!("R4_CONNECTION_PROOF_BUILD_PROFILE");
const BUILD_WORKFLOW_RUN_ID: Option<&str> =
    option_env!("R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ID");
const BUILD_WORKFLOW_RUN_ATTEMPT: Option<&str> =
    option_env!("R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ATTEMPT");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Arguments,
    Filesystem,
    Provenance,
    Database,
    Timeout,
    Assertion,
    Response,
    ControlFile,
}

impl Failure {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Arguments => "proof:fail:arguments",
            Self::Filesystem => "proof:fail:filesystem",
            Self::Provenance => "proof:fail:provenance",
            Self::Database => "proof:fail:database",
            Self::Timeout => "proof:fail:timeout",
            Self::Assertion => "proof:fail:assertion",
            Self::Response => "proof:fail:response",
            Self::ControlFile => "proof:fail:control-file",
        }
    }
}

type ProofResult<T> = Result<T, Failure>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    artifact_root: PathBuf,
    delivery_dir: PathBuf,
    control_dir: PathBuf,
    proof_receipt_sha256: String,
    runner_receipt_sha256: String,
    scratch_nonce: String,
}

#[derive(Clone, Copy)]
struct CompileConstants<'a> {
    git_commit: Option<&'a str>,
    toolchain: Option<&'a str>,
    target: Option<&'a str>,
    profile: Option<&'a str>,
    workflow_run_id: Option<&'a str>,
    workflow_run_attempt: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildOrigin {
    git_commit: String,
    toolchain: String,
    target: String,
    profile: String,
    workflow_run_id: String,
    workflow_run_attempt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BuildReceipt {
    schema: String,
    git_commit: String,
    toolchain: String,
    target: String,
    profile: String,
    workflow_run_id: String,
    workflow_run_attempt: String,
    binary_sha256: String,
    source_hashes: BTreeMap<String, String>,
}

impl BuildReceipt {
    fn validate(&self, schema: &str, origin: &BuildOrigin, binary_sha256: &str) -> bool {
        self.schema == schema
            && self.git_commit == origin.git_commit
            && self.toolchain == origin.toolchain
            && self.target == origin.target
            && self.profile == origin.profile
            && self.workflow_run_id == origin.workflow_run_id
            && self.workflow_run_attempt == origin.workflow_run_attempt
            && self.binary_sha256 == binary_sha256
            && valid_source_hashes(&self.source_hashes)
    }

    fn origin(&self) -> BuildOrigin {
        BuildOrigin {
            git_commit: self.git_commit.clone(),
            toolchain: self.toolchain.clone(),
            target: self.target.clone(),
            profile: self.profile.clone(),
            workflow_run_id: self.workflow_run_id.clone(),
            workflow_run_attempt: self.workflow_run_attempt.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedProvenance {
    origin: BuildOrigin,
    engine: EngineIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScratchObservation {
    current_database: String,
    unix_socket: bool,
    listen_addresses: String,
    cluster_name: String,
    non_template_databases: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReadyReceipt {
    schema: String,
    nonce: String,
    db_name: String,
    run_id: String,
    job_id: String,
    worker_id: String,
}

#[derive(Debug, Serialize)]
struct ProofReport {
    schema: &'static str,
    status: &'static str,
    git_commit: String,
    workflow_run_id: String,
    workflow_run_attempt: String,
    window: ProofWindow,
    result_hashes: ProofHashes,
    checks: ProofChecks,
    counts: ProofCounts,
}

#[derive(Debug, Serialize)]
struct ProofWindow {
    start_date: &'static str,
    end_date: &'static str,
    warmup_sessions: u32,
    evaluation_sessions: u32,
}

#[derive(Debug, Serialize)]
struct ProofHashes {
    illustrative_10bp_per_side: String,
    zero_cost_reference: String,
}

#[derive(Debug, Serialize)]
struct ProofChecks {
    sealed_admission: bool,
    authenticated_enqueue: bool,
    pending_pair_hidden: bool,
    atomic_success: bool,
    whole_pair_binding: bool,
    foreign_api_denied: bool,
    foreign_rls_denied: bool,
    durable_replay: bool,
    rights_revocation: bool,
    scratch_database_absent: bool,
    scratch_artifact_dir_absent: bool,
}

#[derive(Debug, Serialize)]
struct ProofCounts {
    jobs: u32,
    attempts: u32,
    pairs: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnqueueResponse {
    run_id: String,
    job_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorDetail {
    code: String,
    message: String,
    request_id: String,
    #[serde(default)]
    details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairResponse {
    run_id: String,
    status: String,
    illustrative_10bp_per_side: PairProfile,
    zero_cost_reference: PairProfile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairProfile {
    result_content_sha256: String,
    result: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunMetadata {
    id: String,
    job_id: String,
    strategy_config_id: String,
    strategy_id: String,
    strategy_version: String,
    start_date: String,
    end_date: String,
    evaluation_session_count: u32,
    status: String,
    audience: String,
    capability: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    contract_version: String,
    contract_document_sha256: String,
    result_schema_version: String,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    updated_at: String,
    error_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunList {
    items: Vec<RunMetadata>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredPair {
    illustrative_canonical: Vec<u8>,
    illustrative_sha256: String,
    zero_cost_canonical: Vec<u8>,
    zero_cost_sha256: String,
}

#[derive(Debug, sqlx::FromRow)]
struct ObserverRow {
    job_status: String,
    job_owner_user_id: Uuid,
    attempt_count: i32,
    locked_by: Option<String>,
    locked_at: Option<String>,
    run_status: String,
    run_owner_user_id: Uuid,
    run_job_id: Uuid,
    published_attempt_id: Option<Uuid>,
    attempt_id: Uuid,
    attempt_job_id: Uuid,
    attempt_no: i32,
    attempt_outcome: String,
    attempt_claimed_by: Option<String>,
    pair_owner_user_id: Uuid,
    illustrative_canonical: Vec<u8>,
    illustrative_sha256: String,
    zero_cost_canonical: Vec<u8>,
    zero_cost_sha256: String,
}

#[derive(Clone)]
struct CapturedResponse {
    status: StatusCode,
    body: Vec<u8>,
}

pub(super) async fn run(arguments: impl IntoIterator<Item = OsString>) -> ProofResult<()> {
    println!("proof:step:arguments");
    let options = parse_arguments(arguments, cfg!(target_os = "linux"))?;

    println!("proof:step:provenance");
    let provenance = verify_provenance(
        &options,
        CompileConstants {
            git_commit: BUILD_GIT_COMMIT,
            toolchain: BUILD_TOOLCHAIN,
            target: BUILD_TARGET,
            profile: BUILD_PROFILE,
            workflow_run_id: BUILD_WORKFLOW_RUN_ID,
            workflow_run_attempt: BUILD_WORKFLOW_RUN_ATTEMPT,
        },
    )?;

    let deadline = Instant::now() + OVERALL_DEADLINE;
    println!("proof:step:scratch-guard");
    verify_scratch_cluster(deadline, &options.scratch_nonce).await?;

    println!("proof:step:authenticated-admission");
    let report = execute_proof(deadline, &options, &provenance).await?;

    println!("proof:step:report");
    write_report(&options.control_dir, &report)?;
    println!("proof:pass:complete");
    Ok(())
}

fn parse_arguments(
    arguments: impl IntoIterator<Item = OsString>,
    linux: bool,
) -> ProofResult<Options> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref().and_then(OsStr::to_str) != Some("--approved-connection-proof") {
        return Err(Failure::Arguments);
    }

    let mut artifact_root = None;
    let mut delivery_dir = None;
    let mut control_dir = None;
    let mut proof_receipt_sha256 = None;
    let mut runner_receipt_sha256 = None;
    let mut scratch_nonce = None;

    while let Some(option) = arguments.next() {
        let option = option.to_str().ok_or(Failure::Arguments)?;
        let value = arguments.next().ok_or(Failure::Arguments)?;
        let value = value.to_str().ok_or(Failure::Arguments)?;
        match option {
            "--artifact-root" if artifact_root.is_none() => {
                artifact_root = Some(PathBuf::from(value));
            }
            "--delivery-dir" if delivery_dir.is_none() => {
                delivery_dir = Some(PathBuf::from(value));
            }
            "--control-dir" if control_dir.is_none() => {
                control_dir = Some(PathBuf::from(value));
            }
            "--proof-receipt-sha256" if proof_receipt_sha256.is_none() => {
                proof_receipt_sha256 = Some(value.to_owned());
            }
            "--runner-receipt-sha256" if runner_receipt_sha256.is_none() => {
                runner_receipt_sha256 = Some(value.to_owned());
            }
            "--scratch-nonce" if scratch_nonce.is_none() => {
                scratch_nonce = Some(value.to_owned());
            }
            _ => return Err(Failure::Arguments),
        }
    }

    if !linux {
        return Err(Failure::Arguments);
    }
    let options = Options {
        artifact_root: artifact_root.ok_or(Failure::Arguments)?,
        delivery_dir: delivery_dir.ok_or(Failure::Arguments)?,
        control_dir: control_dir.ok_or(Failure::Arguments)?,
        proof_receipt_sha256: proof_receipt_sha256.ok_or(Failure::Arguments)?,
        runner_receipt_sha256: runner_receipt_sha256.ok_or(Failure::Arguments)?,
        scratch_nonce: scratch_nonce.ok_or(Failure::Arguments)?,
    };

    if !is_lower_hex(&options.proof_receipt_sha256, 64)
        || !is_lower_hex(&options.runner_receipt_sha256, 64)
        || !is_lower_hex(&options.scratch_nonce, 32)
    {
        return Err(Failure::Arguments);
    }

    for directory in [
        &options.artifact_root,
        &options.delivery_dir,
        &options.control_dir,
    ] {
        validate_real_directory(directory)?;
    }
    if options.artifact_root.file_name().and_then(OsStr::to_str) != Some(ROOT_BASENAME) {
        return Err(Failure::Arguments);
    }
    if !control_directory_is_private(&options.control_dir)? {
        return Err(Failure::Arguments);
    }
    require_absent(&options.control_dir.join(READY_NAME))?;
    require_absent(&options.control_dir.join(REPORT_NAME))?;
    Ok(options)
}

fn validate_real_directory(path: &Path) -> ProofResult<()> {
    if !path.is_absolute() {
        return Err(Failure::Arguments);
    }
    let mut current = PathBuf::new();
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(Failure::Arguments);
    }
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(_) => current.push(component.as_os_str()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(Failure::Arguments);
            }
        }
        let metadata = fs::symlink_metadata(&current).map_err(|_| Failure::Filesystem)?;
        if metadata.file_type().is_symlink() || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err(Failure::Arguments);
        }
    }
    if !fs::symlink_metadata(path)
        .map_err(|_| Failure::Filesystem)?
        .is_dir()
    {
        return Err(Failure::Arguments);
    }
    Ok(())
}

fn control_directory_is_private(path: &Path) -> ProofResult<bool> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Failure::Filesystem)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        Ok(control_metadata_matches(
            metadata.uid(),
            metadata.permissions().mode(),
            unsafe { libc::geteuid() },
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = metadata;
        Ok(false)
    }
}

fn control_metadata_matches(uid: u32, mode: u32, effective_uid: u32) -> bool {
    uid == effective_uid && mode & 0o7777 == 0o700
}

fn require_absent(path: &Path) -> ProofResult<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(Failure::Filesystem),
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn positive_ascii_number(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

fn safe_relative_source_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}

fn valid_source_hashes(source_hashes: &BTreeMap<String, String>) -> bool {
    !source_hashes.is_empty()
        && source_hashes
            .iter()
            .all(|(path, hash)| safe_relative_source_path(path) && is_lower_hex(hash, 64))
}

fn compile_origin(constants: CompileConstants<'_>) -> ProofResult<BuildOrigin> {
    let git_commit = constants.git_commit.ok_or(Failure::Provenance)?;
    let toolchain = constants.toolchain.ok_or(Failure::Provenance)?;
    let target = constants.target.ok_or(Failure::Provenance)?;
    let profile = constants.profile.ok_or(Failure::Provenance)?;
    let workflow_run_id = constants.workflow_run_id.ok_or(Failure::Provenance)?;
    let workflow_run_attempt = constants.workflow_run_attempt.ok_or(Failure::Provenance)?;
    if !is_lower_hex(git_commit, 40)
        || target != "x86_64-unknown-linux-musl"
        || profile != "release"
        || !safe_build_token(toolchain)
        || !positive_ascii_number(workflow_run_id)
        || !positive_ascii_number(workflow_run_attempt)
    {
        return Err(Failure::Provenance);
    }
    Ok(BuildOrigin {
        git_commit: git_commit.to_owned(),
        toolchain: toolchain.to_owned(),
        target: target.to_owned(),
        profile: profile.to_owned(),
        workflow_run_id: workflow_run_id.to_owned(),
        workflow_run_attempt: workflow_run_attempt.to_owned(),
    })
}

fn safe_build_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn verify_provenance(
    options: &Options,
    constants: CompileConstants<'_>,
) -> ProofResult<VerifiedProvenance> {
    let origin = compile_origin(constants)?;
    let proof_path = options.delivery_dir.join(PROOF_RECEIPT_NAME);
    let runner_path = options.delivery_dir.join(RUNNER_RECEIPT_NAME);
    let binary_path = options.delivery_dir.join(RUNNER_NAME);

    let proof_bytes = read_regular_file_bounded(&proof_path, MAX_RECEIPT_BYTES)?;
    if sha256_hex(&proof_bytes) != options.proof_receipt_sha256 {
        return Err(Failure::Provenance);
    }
    let proof_receipt = parse_receipt(&proof_bytes)?;
    let current_binary_sha256 = hash_current_executable()?;
    if !proof_receipt.validate(PROOF_SCHEMA, &origin, &current_binary_sha256) {
        return Err(Failure::Provenance);
    }

    let runner_receipt_bytes = read_regular_file_bounded(&runner_path, MAX_RECEIPT_BYTES)?;
    if sha256_hex(&runner_receipt_bytes) != options.runner_receipt_sha256 {
        return Err(Failure::Provenance);
    }
    let runner_receipt = parse_receipt(&runner_receipt_bytes)?;
    let runner_binary_sha256 = hash_regular_file(&binary_path)?;
    if !runner_receipt.validate(
        RUNNER_SCHEMA,
        &proof_receipt.origin(),
        &runner_binary_sha256,
    ) {
        return Err(Failure::Provenance);
    }

    let engine = EngineIdentity::new(
        &runner_receipt.git_commit,
        &format!("sha256:{runner_binary_sha256}"),
        &runner_receipt.toolchain,
        &runner_receipt.target,
        &runner_receipt.profile,
    )
    .map_err(|_| Failure::Provenance)?;
    Ok(VerifiedProvenance { origin, engine })
}

fn parse_receipt(bytes: &[u8]) -> ProofResult<BuildReceipt> {
    let value = strict_json_value(bytes).map_err(|_| Failure::Provenance)?;
    serde_json::from_value(value).map_err(|_| Failure::Provenance)
}

fn read_regular_file_bounded(path: &Path, limit: usize) -> ProofResult<Vec<u8>> {
    let file = open_regular_file(path)?;
    let metadata = file.metadata().map_err(|_| Failure::Filesystem)?;
    if metadata.len() > limit as u64 {
        return Err(Failure::Filesystem);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Filesystem)?;
    if bytes.len() > limit {
        return Err(Failure::Filesystem);
    }
    Ok(bytes)
}

fn hash_regular_file(path: &Path) -> ProofResult<String> {
    let mut file = open_regular_file(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| Failure::Filesystem)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn hash_current_executable() -> ProofResult<String> {
    let path = env::current_exe().map_err(|_| Failure::Provenance)?;
    hash_regular_file(&path).map_err(|_| Failure::Provenance)
}

fn open_regular_file(path: &Path) -> ProofResult<File> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| Failure::Filesystem)?;
        if !file.metadata().map_err(|_| Failure::Filesystem)?.is_file() {
            return Err(Failure::Filesystem);
        }
        Ok(file)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Failure::Filesystem)
    }
}

// job-queue keeps parse_strict_json crate-private. Receipts need duplicate-aware
// decoding at this binary boundary; typed simulation payload/results still go
// through their public from_json contract methods below.
#[derive(Debug)]
struct StrictJson(Value);

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> serde::de::Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(|number| StrictJson(Value::Number(number)))
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictJson>()? {
            values.push(value.0);
        }
        Ok(StrictJson(Value::Array(values)))
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            let value = entries.next_value::<StrictJson>()?;
            object.insert(key, value.0);
        }
        Ok(StrictJson(Value::Object(object)))
    }
}

fn strict_json_value(bytes: &[u8]) -> ProofResult<Value> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJson::deserialize(&mut deserializer)
        .map_err(|_| Failure::Response)?
        .0;
    deserializer.end().map_err(|_| Failure::Response)?;
    Ok(value)
}

fn closed_json<T: DeserializeOwned>(bytes: &[u8]) -> ProofResult<T> {
    serde_json::from_value(strict_json_value(bytes)?).map_err(|_| Failure::Response)
}

async fn bounded_for<T>(
    deadline: Instant,
    operation_timeout: Duration,
    future: impl Future<Output = T>,
) -> ProofResult<T> {
    let operation_deadline = (Instant::now() + operation_timeout).min(deadline);
    tokio::time::timeout_at(operation_deadline.into(), future)
        .await
        .map_err(|_| Failure::Timeout)
}

fn scratch_guard_accepts(
    database_url: &str,
    nonce: &str,
    observation: &ScratchObservation,
) -> bool {
    database_url == DATABASE_URL
        && observation.current_database == "postgres"
        && observation.unix_socket
        && observation.listen_addresses.is_empty()
        && observation.cluster_name == format!("r4-proof-{nonce}")
        && observation.non_template_databases.len() == 1
        && observation.non_template_databases[0] == "postgres"
}

async fn readonly_super_pool(deadline: Instant) -> ProofResult<sqlx::PgPool> {
    let database_url = env::var("DATABASE_URL").map_err(|_| Failure::Database)?;
    if database_url != DATABASE_URL {
        return Err(Failure::Database);
    }
    let options = DATABASE_URL
        .parse::<sqlx::postgres::PgConnectOptions>()
        .map_err(|_| Failure::Database)?
        .options([("default_transaction_read_only", "on")])
        .log_statements(tracing::log::LevelFilter::Off);
    bounded_for(
        deadline,
        CONNECTION_TIMEOUT,
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(CONNECTION_TIMEOUT)
            .connect_with(options),
    )
    .await?
    .map_err(|_| Failure::Database)
}

async fn read_scratch_observation(
    pool: &sqlx::PgPool,
    deadline: Instant,
) -> ProofResult<ScratchObservation> {
    let row = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query(
            "SELECT current_database(), inet_server_addr() IS NULL,
                    current_setting('listen_addresses'), current_setting('cluster_name')",
        )
        .fetch_one(pool),
    )
    .await?
    .map_err(|_| Failure::Database)?;
    let current_database: String = row.try_get(0).map_err(|_| Failure::Database)?;
    let unix_socket: bool = row.try_get(1).map_err(|_| Failure::Database)?;
    let listen_addresses: String = row.try_get(2).map_err(|_| Failure::Database)?;
    let cluster_name: String = row.try_get(3).map_err(|_| Failure::Database)?;
    let non_template_databases = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_scalar::<_, String>(
            "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY datname",
        )
        .fetch_all(pool),
    )
    .await?
    .map_err(|_| Failure::Database)?;
    Ok(ScratchObservation {
        current_database,
        unix_socket,
        listen_addresses,
        cluster_name,
        non_template_databases,
    })
}

async fn verify_scratch_cluster(deadline: Instant, nonce: &str) -> ProofResult<()> {
    let pool = readonly_super_pool(deadline).await?;
    let observation = read_scratch_observation(&pool, deadline).await?;
    bounded_for(deadline, CONNECTION_TIMEOUT, pool.close()).await?;
    if !scratch_guard_accepts(DATABASE_URL, nonce, &observation) {
        return Err(Failure::Database);
    }
    Ok(())
}

async fn verify_scratch_database_absent(
    deadline: Instant,
    nonce: &str,
    db_name: &str,
) -> ProofResult<bool> {
    let pool = readonly_super_pool(deadline).await?;
    let observation = read_scratch_observation(&pool, deadline).await?;
    if !scratch_guard_accepts(DATABASE_URL, nonce, &observation) {
        bounded_for(deadline, CONNECTION_TIMEOUT, pool.close()).await?;
        return Err(Failure::Database);
    }
    let absent = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_scalar::<_, bool>(
            "SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
        )
        .bind(db_name)
        .fetch_one(&pool),
    )
    .await?
    .map_err(|_| Failure::Database)?;
    bounded_for(deadline, CONNECTION_TIMEOUT, pool.close()).await?;
    Ok(absent)
}

async fn role_pool(
    url: &str,
    actor_user_id: Option<Uuid>,
    deadline: Instant,
) -> ProofResult<sqlx::PgPool> {
    let mut options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .map_err(|_| Failure::Database)?
        .log_statements(tracing::log::LevelFilter::Off);
    if let Some(actor_user_id) = actor_user_id {
        options = options.options([("app.actor_user_id", actor_user_id.to_string())]);
    }
    bounded_for(
        deadline,
        CONNECTION_TIMEOUT,
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(CONNECTION_TIMEOUT)
            .connect_with(options),
    )
    .await?
    .map_err(|_| Failure::Database)
}

async fn execute_proof(
    deadline: Instant,
    options: &Options,
    provenance: &VerifiedProvenance,
) -> ProofResult<ProofReport> {
    let mut harness = bounded_for(deadline, REQUEST_TIMEOUT, Harness::new())
        .await?
        .ok_or(Failure::Database)?;
    bounded_for(
        deadline,
        QUERY_TIMEOUT,
        harness.seed_simulation_backtest_rights(),
    )
    .await?;
    let foreign_owner = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        harness.seed_user(
            auth::entitlement::Role::Owner,
            "r4-proof-foreign-owner@lagrange.test",
            "r4-proof-foreign-issuer",
            "r4-proof-foreign-subject",
        ),
    )
    .await?;
    let strategy_config_id = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        harness.seed_simulation_strategy_config(&harness.owner),
    )
    .await?;

    let mut api_config = (*harness.state().cfg).clone();
    api_config.code_commit = provenance.origin.git_commit.clone();
    api_config.owner_beta_access = api_server::http::state::OwnerBetaAccessMode::OwnerOnly;
    api_config.owner_beta_paper = api_server::http::state::OwnerBetaPaperMode::Disabled;
    api_config.owner_beta_price_input = api_server::http::state::OwnerBetaPriceInputMode::SealedV1;
    api_config.owner_beta_daily =
        job_queue::owner_beta::runner::daily_runtime::DailyProcessMode::Disabled;
    api_config.owner_beta_equity_signals =
        api_server::http::state::OwnerBetaEquitySignalsMode::Disabled;
    api_config.owner_equity_v2_pins = None;
    api_config.owner_equity_v2_api_artifact_root = None;
    api_config.owner_intraday_quotes =
        api_server::http::state::OwnerIntradayQuoteReadConfig::Disabled;
    api_config.owner_market_stream =
        api_server::http::owner_market_stream_config::MarketStreamReadConfig::Disabled;
    api_config.artifact_root = options
        .artifact_root
        .parent()
        .ok_or(Failure::Arguments)?
        .to_path_buf();
    let replay_config = api_config.clone();
    let state = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        api_server::http::state::ApiState::from_pools(
            api_config,
            harness.app_pool.clone(),
            harness.admin_pool.clone(),
            harness.audit_pool.clone(),
        ),
    )
    .await?
    .map_err(|_| Failure::Database)?;
    harness.app = api_server::http::api_router(state.clone());

    let owner_pool = role_pool(&harness.app_url, Some(harness.owner.user_id), deadline).await?;
    let foreign_pool = role_pool(&harness.app_url, Some(foreign_owner.user_id), deadline).await?;

    let request_body = serde_json::json!({
        "strategy_config_id": strategy_config_id,
        "start_date": START_DATE,
        "end_date": END_DATE,
    });
    let enqueued = send_request(
        &harness,
        deadline,
        "POST",
        RUNS_PATH,
        Some(&harness.owner),
        true,
        Some(IDEMPOTENCY_KEY),
        Some(request_body.clone()),
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    if enqueued.status != StatusCode::ACCEPTED {
        return Err(Failure::Response);
    }
    let enqueue: EnqueueResponse = closed_json(&enqueued.body)?;
    if enqueue.status != "PENDING" {
        return Err(Failure::Assertion);
    }
    let run_id = Uuid::parse_str(&enqueue.run_id).map_err(|_| Failure::Response)?;
    let job_id = Uuid::parse_str(&enqueue.job_id).map_err(|_| Failure::Response)?;

    let payload = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_scalar::<_, String>(
            "SELECT payload_json::text FROM public.jobs
              WHERE id = $1 AND owner_user_id = $2
                AND job_type = 'owner_beta_price_only_simulation'",
        )
        .bind(job_id)
        .bind(harness.owner.user_id)
        .fetch_optional(&owner_pool),
    )
    .await?
    .map_err(|_| Failure::Database)?
    .ok_or(Failure::Assertion)?;
    let input =
        SimulationJobInput::from_json(payload.as_bytes()).map_err(|_| Failure::Assertion)?;
    let schedule = input.schedule();
    if input.run_id() != run_id
        || input.strategy_config_id() != strategy_config_id
        || input.evaluation_start().to_iso() != START_DATE
        || input.evaluation_end().to_iso() != END_DATE
        || schedule.warmup_sessions().len() != 200
        || schedule.evaluation_sessions().len() != 2
        || schedule.evaluation_sessions()[0].to_iso() != START_DATE
        || schedule.evaluation_sessions()[1].to_iso() != END_DATE
    {
        return Err(Failure::Assertion);
    }

    let hidden_pair = send_request(
        &harness,
        deadline,
        "GET",
        &pair_path(run_id),
        Some(&harness.owner),
        false,
        None,
        None,
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    let hidden_body = parse_error(
        &hidden_pair,
        StatusCode::CONFLICT,
        "SIMULATION_RESULT_NOT_READY",
    )?;
    if hidden_body.get("illustrative_10bp_per_side").is_some()
        || hidden_body.get("zero_cost_reference").is_some()
    {
        return Err(Failure::Assertion);
    }

    verify_foreign_api_denial(&harness, deadline, &foreign_owner, run_id).await?;
    verify_rls_absence(
        &harness,
        deadline,
        &owner_pool,
        &foreign_pool,
        run_id,
        false,
    )
    .await?;

    let worker_id = format!("r4-proof-{}", options.scratch_nonce);
    let readiness = ReadyReceipt {
        schema: READY_SCHEMA.to_owned(),
        nonce: options.scratch_nonce.clone(),
        db_name: harness.db_name.clone(),
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        worker_id: worker_id.clone(),
    };
    write_ready(&options.control_dir, &readiness)?;
    println!("proof:ready:runner-may-start");

    let succeeded =
        poll_for_success(&harness, deadline, run_id, job_id, strategy_config_id).await?;
    if succeeded.status != "SUCCEEDED" {
        return Err(Failure::Assertion);
    }

    let pair_response = send_request(
        &harness,
        deadline,
        "GET",
        &pair_path(run_id),
        Some(&harness.owner),
        false,
        None,
        None,
        MAX_PAIR_RESPONSE_BYTES,
    )
    .await?;
    if pair_response.status != StatusCode::OK {
        return Err(Failure::Response);
    }
    let pair: PairResponse = closed_json(&pair_response.body)?;
    if pair.run_id != run_id.to_string() || pair.status != "SUCCEEDED" {
        return Err(Failure::Assertion);
    }

    let stored_pair = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_as::<_, StoredPair>(
            "SELECT illustrative_canonical, illustrative_sha256,
                    zero_cost_canonical, zero_cost_sha256
               FROM public.owner_beta_simulation_result_pairs
              WHERE run_id = $1 AND owner_user_id = $2",
        )
        .bind(run_id)
        .bind(harness.owner.user_id)
        .fetch_optional(&owner_pool),
    )
    .await?
    .map_err(|_| Failure::Database)?
    .ok_or(Failure::Assertion)?;
    let result_hashes = verify_whole_pair(&pair, &stored_pair, &input, &provenance.engine)?;

    let worker_pool = role_pool(&harness.worker_url, None, deadline).await?;
    verify_observer_success(
        &worker_pool,
        deadline,
        run_id,
        job_id,
        harness.owner.user_id,
        &worker_id,
        &stored_pair,
    )
    .await?;

    verify_foreign_api_denial(&harness, deadline, &foreign_owner, run_id).await?;
    verify_rls_absence(&harness, deadline, &owner_pool, &foreign_pool, run_id, true).await?;
    verify_owner_list(&harness, deadline, run_id, job_id, strategy_config_id).await?;

    // Rebuilding the real state drops its in-memory idempotency cache. The
    // replay must therefore resolve through the durable database record.
    harness.app = axum::Router::new();
    drop(state);
    let replay_state = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        api_server::http::state::ApiState::from_pools(
            replay_config,
            harness.app_pool.clone(),
            harness.admin_pool.clone(),
            harness.audit_pool.clone(),
        ),
    )
    .await?
    .map_err(|_| Failure::Database)?;
    harness.app = api_server::http::api_router(replay_state.clone());
    drop(replay_state);
    let replay = send_request(
        &harness,
        deadline,
        "POST",
        RUNS_PATH,
        Some(&harness.owner),
        true,
        Some(IDEMPOTENCY_KEY),
        Some(request_body),
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    if replay.status != StatusCode::ACCEPTED {
        return Err(Failure::Response);
    }
    let replay_body: EnqueueResponse = closed_json(&replay.body)?;
    if replay_body.run_id != enqueue.run_id
        || replay_body.job_id != enqueue.job_id
        || replay_body.status != "PENDING"
    {
        return Err(Failure::Assertion);
    }
    verify_observer_success(
        &worker_pool,
        deadline,
        run_id,
        job_id,
        harness.owner.user_id,
        &worker_id,
        &stored_pair,
    )
    .await?;

    bounded_for(
        deadline,
        QUERY_TIMEOUT,
        harness.revoke_simulation_backtest_rights(),
    )
    .await?;
    let owner_after_revocation = send_request(
        &harness,
        deadline,
        "GET",
        &run_path(run_id),
        Some(&harness.owner),
        false,
        None,
        None,
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    let revocation_error = parse_error(&owner_after_revocation, StatusCode::FORBIDDEN, "")?;
    if revocation_error.get("error").is_none() {
        return Err(Failure::Assertion);
    }
    let foreign_after_revocation = send_request(
        &harness,
        deadline,
        "GET",
        &run_path(run_id),
        Some(&foreign_owner),
        false,
        None,
        None,
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    parse_error(
        &foreign_after_revocation,
        StatusCode::NOT_FOUND,
        "RESOURCE_NOT_FOUND",
    )?;

    let scratch_db_name = harness.db_name.clone();
    let scratch_artifact_root = harness.artifact_root.clone();
    harness.app = axum::Router::new();
    drop(owner_pool);
    drop(foreign_pool);
    drop(worker_pool);
    bounded_for(deadline, REQUEST_TIMEOUT, harness.teardown()).await?;
    let scratch_artifact_dir_absent = !scratch_artifact_root.exists();
    let scratch_database_absent =
        verify_scratch_database_absent(deadline, &options.scratch_nonce, &scratch_db_name).await?;
    if !scratch_artifact_dir_absent || !scratch_database_absent {
        return Err(Failure::Assertion);
    }

    Ok(ProofReport {
        schema: REPORT_SCHEMA,
        status: "PASS",
        git_commit: provenance.origin.git_commit.clone(),
        workflow_run_id: provenance.origin.workflow_run_id.clone(),
        workflow_run_attempt: provenance.origin.workflow_run_attempt.clone(),
        window: ProofWindow {
            start_date: START_DATE,
            end_date: END_DATE,
            warmup_sessions: 200,
            evaluation_sessions: 2,
        },
        result_hashes: ProofHashes {
            illustrative_10bp_per_side: result_hashes.0,
            zero_cost_reference: result_hashes.1,
        },
        checks: ProofChecks {
            sealed_admission: true,
            authenticated_enqueue: true,
            pending_pair_hidden: true,
            atomic_success: true,
            whole_pair_binding: true,
            foreign_api_denied: true,
            foreign_rls_denied: true,
            durable_replay: true,
            rights_revocation: true,
            scratch_database_absent,
            scratch_artifact_dir_absent,
        },
        counts: ProofCounts {
            jobs: 1,
            attempts: 1,
            pairs: 1,
        },
    })
}

// Keep the shared Harness::send request fields explicit alongside the proof's
// deadline and response bound, as in the harness's own request helpers.
#[allow(clippy::too_many_arguments)]
async fn send_request(
    harness: &Harness,
    deadline: Instant,
    method: &str,
    path: &str,
    user: Option<&UserCtx>,
    csrf: bool,
    idempotency_key: Option<&str>,
    body: Option<Value>,
    max_body_bytes: usize,
) -> ProofResult<CapturedResponse> {
    let response = bounded_for(
        deadline,
        REQUEST_TIMEOUT,
        harness.send(
            method,
            path,
            user,
            csrf,
            Some(REQUEST_ID),
            idempotency_key,
            body,
        ),
    )
    .await?;
    let status = response.status();
    if response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        != Some("no-store")
        || response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            != Some(REQUEST_ID)
    {
        return Err(Failure::Response);
    }

    let mut body = response.into_body();
    let mut bytes = Vec::new();
    loop {
        let frame = bounded_for(deadline, BODY_FRAME_TIMEOUT, body.frame()).await?;
        match frame {
            None => break,
            Some(Err(_)) => return Err(Failure::Response),
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    let next_length = bytes
                        .len()
                        .checked_add(data.len())
                        .ok_or(Failure::Response)?;
                    if next_length > max_body_bytes {
                        return Err(Failure::Response);
                    }
                    bytes.extend_from_slice(&data);
                }
            }
        }
    }
    Ok(CapturedResponse {
        status,
        body: bytes,
    })
}

fn parse_error(
    response: &CapturedResponse,
    status: StatusCode,
    expected_code: &str,
) -> ProofResult<Value> {
    if response.status != status {
        return Err(Failure::Response);
    }
    let value = strict_json_value(&response.body)?;
    let envelope: ErrorEnvelope =
        serde_json::from_value(value.clone()).map_err(|_| Failure::Response)?;
    if envelope.error.request_id != REQUEST_ID
        || envelope.error.message.is_empty()
        || envelope.error.details.is_some()
        || (!expected_code.is_empty() && envelope.error.code != expected_code)
    {
        return Err(Failure::Response);
    }
    Ok(value)
}

fn run_path(run_id: Uuid) -> String {
    format!("{RUNS_PATH}/{run_id}")
}

fn pair_path(run_id: Uuid) -> String {
    format!("{}/pair", run_path(run_id))
}

fn cancel_path(run_id: Uuid) -> String {
    format!("{}/cancel", run_path(run_id))
}

async fn verify_foreign_api_denial(
    harness: &Harness,
    deadline: Instant,
    foreign_owner: &UserCtx,
    run_id: Uuid,
) -> ProofResult<()> {
    let missing_id = Uuid::new_v4();
    for (path, missing_path) in [
        (run_path(run_id), run_path(missing_id)),
        (pair_path(run_id), pair_path(missing_id)),
    ] {
        let foreign = send_request(
            harness,
            deadline,
            "GET",
            &path,
            Some(foreign_owner),
            false,
            None,
            None,
            MAX_SMALL_RESPONSE_BYTES,
        )
        .await?;
        let foreign_body = parse_error(&foreign, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND")?;
        let missing = send_request(
            harness,
            deadline,
            "GET",
            &missing_path,
            Some(foreign_owner),
            false,
            None,
            None,
            MAX_SMALL_RESPONSE_BYTES,
        )
        .await?;
        let missing_body = parse_error(&missing, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND")?;
        if error_code(&foreign_body)? != error_code(&missing_body)? {
            return Err(Failure::Assertion);
        }
    }

    let foreign_cancel = send_request(
        harness,
        deadline,
        "POST",
        &cancel_path(run_id),
        Some(foreign_owner),
        true,
        Some(DENIAL_IDEMPOTENCY_KEY),
        Some(serde_json::json!({})),
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    let foreign_body = parse_error(&foreign_cancel, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND")?;
    let missing_cancel = send_request(
        harness,
        deadline,
        "POST",
        &cancel_path(Uuid::new_v4()),
        Some(foreign_owner),
        true,
        Some(DENIAL_IDEMPOTENCY_KEY),
        Some(serde_json::json!({})),
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    let missing_body = parse_error(&missing_cancel, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND")?;
    if error_code(&foreign_body)? != error_code(&missing_body)? {
        return Err(Failure::Assertion);
    }

    let list = send_request(
        harness,
        deadline,
        "GET",
        RUNS_PATH,
        Some(foreign_owner),
        false,
        None,
        None,
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    if list.status != StatusCode::OK {
        return Err(Failure::Response);
    }
    let list: RunList = closed_json(&list.body)?;
    if !list.items.is_empty() || list.next_cursor.is_some() || list.has_more {
        return Err(Failure::Assertion);
    }
    Ok(())
}

fn error_code(value: &Value) -> ProofResult<&str> {
    value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .ok_or(Failure::Response)
}

async fn rls_counts(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    deadline: Instant,
) -> ProofResult<(i64, i64)> {
    bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT
                (SELECT count(*)::bigint FROM public.owner_beta_simulation_runs WHERE id = $1),
                (SELECT count(*)::bigint FROM public.owner_beta_simulation_result_pairs WHERE run_id = $1)",
        )
        .bind(run_id)
        .fetch_one(pool),
    )
    .await?
    .map_err(|_| Failure::Database)
}

async fn verify_rls_absence(
    harness: &Harness,
    deadline: Instant,
    owner_pool: &sqlx::PgPool,
    foreign_pool: &sqlx::PgPool,
    run_id: Uuid,
    after_success: bool,
) -> ProofResult<()> {
    let foreign = rls_counts(foreign_pool, run_id, deadline).await?;
    let unscoped = rls_counts(&harness.app_pool, run_id, deadline).await?;
    if foreign != (0, 0) || unscoped != (0, 0) {
        return Err(Failure::Assertion);
    }
    if after_success && rls_counts(owner_pool, run_id, deadline).await? != (1, 1) {
        return Err(Failure::Assertion);
    }
    Ok(())
}

async fn poll_for_success(
    harness: &Harness,
    deadline: Instant,
    run_id: Uuid,
    job_id: Uuid,
    strategy_config_id: Uuid,
) -> ProofResult<RunMetadata> {
    loop {
        let response = send_request(
            harness,
            deadline,
            "GET",
            &run_path(run_id),
            Some(&harness.owner),
            false,
            None,
            None,
            MAX_SMALL_RESPONSE_BYTES,
        )
        .await?;
        if response.status != StatusCode::OK {
            return Err(Failure::Response);
        }
        let metadata: RunMetadata = closed_json(&response.body)?;
        if !valid_metadata(&metadata, run_id, job_id, strategy_config_id) {
            return Err(Failure::Assertion);
        }
        match metadata.status.as_str() {
            "SUCCEEDED" => return Ok(metadata),
            "PENDING" | "RUNNING" => {
                bounded_for(
                    deadline,
                    Duration::from_secs(1),
                    tokio::time::sleep(Duration::from_secs(1)),
                )
                .await?;
            }
            _ => return Err(Failure::Assertion),
        }
    }
}

fn valid_metadata(
    metadata: &RunMetadata,
    run_id: Uuid,
    job_id: Uuid,
    strategy_config_id: Uuid,
) -> bool {
    let state_times_valid = match metadata.status.as_str() {
        "PENDING" => metadata.started_at.is_none() && metadata.finished_at.is_none(),
        "RUNNING" => metadata.finished_at.is_none(),
        "SUCCEEDED" => metadata.started_at.is_some() && metadata.finished_at.is_some(),
        _ => false,
    };
    metadata.id == run_id.to_string()
        && metadata.job_id == job_id.to_string()
        && metadata.strategy_config_id == strategy_config_id.to_string()
        && metadata.strategy_id == "trend_following"
        && metadata.strategy_version == "1.0.0"
        && metadata.start_date == START_DATE
        && metadata.end_date == END_DATE
        && metadata.evaluation_session_count == 2
        && metadata.audience == "OWNER_ONLY"
        && metadata.capability == "PRICE_RETURN_ONLY"
        && metadata.vendor_snapshot
        && !metadata.strict_pit
        && metadata.contract_version == job_queue::owner_beta::simulation::CONTRACT_VERSION
        && is_sha256_prefixed(&metadata.contract_document_sha256)
        && metadata.result_schema_version
            == job_queue::owner_beta::simulation::RESULT_SCHEMA_VERSION
        && !metadata.created_at.is_empty()
        && !metadata.updated_at.is_empty()
        && metadata.error_code.is_none()
        && state_times_valid
}

fn is_sha256_prefixed(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|digest| is_lower_hex(digest, 64))
}

async fn verify_owner_list(
    harness: &Harness,
    deadline: Instant,
    run_id: Uuid,
    job_id: Uuid,
    strategy_config_id: Uuid,
) -> ProofResult<()> {
    let response = send_request(
        harness,
        deadline,
        "GET",
        RUNS_PATH,
        Some(&harness.owner),
        false,
        None,
        None,
        MAX_SMALL_RESPONSE_BYTES,
    )
    .await?;
    if response.status != StatusCode::OK {
        return Err(Failure::Response);
    }
    let list: RunList = closed_json(&response.body)?;
    if list.items.len() != 1
        || list.has_more
        || list.next_cursor.is_some()
        || !valid_metadata(&list.items[0], run_id, job_id, strategy_config_id)
        || list.items[0].status != "SUCCEEDED"
    {
        return Err(Failure::Assertion);
    }
    Ok(())
}

fn verify_whole_pair(
    response: &PairResponse,
    stored: &StoredPair,
    input: &SimulationJobInput,
    expected_engine: &EngineIdentity,
) -> ProofResult<(String, String)> {
    let illustrative_input = serde_json::to_vec(&response.illustrative_10bp_per_side.result)
        .map_err(|_| Failure::Response)?;
    let zero_cost_input =
        serde_json::to_vec(&response.zero_cost_reference.result).map_err(|_| Failure::Response)?;
    let illustrative =
        SimulationResult::from_json(&illustrative_input).map_err(|_| Failure::Assertion)?;
    let zero_cost =
        SimulationResult::from_json(&zero_cost_input).map_err(|_| Failure::Assertion)?;
    let illustrative_canonical = illustrative
        .canonical_json()
        .map_err(|_| Failure::Assertion)?;
    let zero_cost_canonical = zero_cost.canonical_json().map_err(|_| Failure::Assertion)?;
    let illustrative_hash = format!("sha256:{}", sha256_hex(&illustrative_canonical));
    let zero_cost_hash = format!("sha256:{}", sha256_hex(&zero_cost_canonical));
    if illustrative_canonical != stored.illustrative_canonical
        || zero_cost_canonical != stored.zero_cost_canonical
        || illustrative_hash != stored.illustrative_sha256
        || zero_cost_hash != stored.zero_cost_sha256
        || illustrative_hash != response.illustrative_10bp_per_side.result_content_sha256
        || zero_cost_hash != response.zero_cost_reference.result_content_sha256
    {
        return Err(Failure::Assertion);
    }
    let _checked = CheckedSimulationPair::from_canonical(
        &stored.illustrative_canonical,
        &stored.zero_cost_canonical,
        input,
        expected_engine,
    )
    .map_err(|_| Failure::Assertion)?;
    Ok((illustrative_hash, zero_cost_hash))
}

async fn verify_observer_success(
    pool: &sqlx::PgPool,
    deadline: Instant,
    run_id: Uuid,
    job_id: Uuid,
    owner_user_id: Uuid,
    worker_id: &str,
    expected_pair: &StoredPair,
) -> ProofResult<()> {
    let mut connection = bounded_for(deadline, CONNECTION_TIMEOUT, pool.acquire())
        .await?
        .map_err(|_| Failure::Database)?;
    let mut transaction = bounded_for(deadline, QUERY_TIMEOUT, connection.begin())
        .await?
        .map_err(|_| Failure::Database)?;
    bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query("SET TRANSACTION READ ONLY").execute(&mut *transaction),
    )
    .await?
    .map_err(|_| Failure::Database)?;

    let row = bounded_for(
        deadline,
        QUERY_TIMEOUT,
        sqlx::query_as::<_, ObserverRow>(
            "SELECT j.status AS job_status, j.attempt_count,
                    j.owner_user_id AS job_owner_user_id,
                    j.locked_by, j.locked_at::text AS locked_at,
                    r.status AS run_status, r.owner_user_id AS run_owner_user_id,
                    r.job_id AS run_job_id, r.published_attempt_id,
                    a.id AS attempt_id, a.job_id AS attempt_job_id,
                    a.attempt_no, a.outcome AS attempt_outcome,
                    a.claimed_by AS attempt_claimed_by,
                    p.owner_user_id AS pair_owner_user_id,
                    p.illustrative_canonical, p.illustrative_sha256,
                    p.zero_cost_canonical, p.zero_cost_sha256
               FROM public.jobs AS j
               JOIN public.owner_beta_simulation_runs AS r ON r.job_id = j.id
               JOIN public.job_attempts AS a ON a.job_id = j.id
               JOIN public.owner_beta_simulation_result_pairs AS p ON p.run_id = r.id
              WHERE j.id = $1 AND r.id = $2",
        )
        .bind(job_id)
        .bind(run_id)
        .fetch_optional(&mut *transaction),
    )
    .await?
    .map_err(|_| Failure::Database)?
    .ok_or(Failure::Assertion)?;

    let (job_count, attempt_count, succeeded_attempt_count, pair_count): (i64, i64, i64, i64) =
        bounded_for(
            deadline,
            QUERY_TIMEOUT,
            sqlx::query_as(
                "SELECT
                    (SELECT count(*)::bigint FROM public.jobs
                      WHERE owner_user_id = $1 AND job_type = 'owner_beta_price_only_simulation'),
                    (SELECT count(*)::bigint FROM public.job_attempts WHERE job_id = $2),
                    (SELECT count(*)::bigint FROM public.job_attempts
                      WHERE job_id = $2 AND outcome = 'SUCCEEDED'),
                    (SELECT count(*)::bigint FROM public.owner_beta_simulation_result_pairs
                      WHERE run_id = $3)",
            )
            .bind(owner_user_id)
            .bind(job_id)
            .bind(run_id)
            .fetch_one(&mut *transaction),
        )
        .await?
        .map_err(|_| Failure::Database)?;

    if row.job_status != "SUCCEEDED"
        || row.job_owner_user_id != owner_user_id
        || row.attempt_count != 1
        || row.locked_by.is_some()
        || row.locked_at.is_some()
        || row.run_status != "SUCCEEDED"
        || row.run_owner_user_id != owner_user_id
        || row.run_job_id != job_id
        || row.published_attempt_id != Some(row.attempt_id)
        || row.attempt_job_id != job_id
        || row.attempt_no != 1
        || row.attempt_outcome != "SUCCEEDED"
        || row.attempt_claimed_by.as_deref() != Some(worker_id)
        || row.pair_owner_user_id != owner_user_id
        || row.illustrative_canonical != expected_pair.illustrative_canonical
        || row.illustrative_sha256 != expected_pair.illustrative_sha256
        || row.zero_cost_canonical != expected_pair.zero_cost_canonical
        || row.zero_cost_sha256 != expected_pair.zero_cost_sha256
        || (
            job_count,
            attempt_count,
            succeeded_attempt_count,
            pair_count,
        ) != (1, 1, 1, 1)
    {
        return Err(Failure::Assertion);
    }
    bounded_for(deadline, QUERY_TIMEOUT, transaction.commit())
        .await?
        .map_err(|_| Failure::Database)?;
    Ok(())
}

fn write_ready(control_dir: &Path, receipt: &ReadyReceipt) -> ProofResult<()> {
    if receipt.schema != READY_SCHEMA
        || !is_lower_hex(&receipt.nonce, 32)
        || Uuid::parse_str(&receipt.run_id).is_err()
        || Uuid::parse_str(&receipt.job_id).is_err()
        || receipt.worker_id != format!("r4-proof-{}", receipt.nonce)
        || receipt.db_name.is_empty()
        || !receipt
            .db_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Failure::ControlFile);
    }
    let mut bytes = serde_json::to_vec(receipt).map_err(|_| Failure::ControlFile)?;
    bytes.push(b'\n');
    write_private_new(&control_dir.join(READY_NAME), control_dir, &bytes)
}

fn write_report(control_dir: &Path, report: &ProofReport) -> ProofResult<()> {
    validate_real_directory(control_dir).map_err(|_| Failure::ControlFile)?;
    if !control_directory_is_private(control_dir).map_err(|_| Failure::ControlFile)?
        || report.schema != REPORT_SCHEMA
        || report.status != "PASS"
        || report.window.start_date != START_DATE
        || report.window.end_date != END_DATE
        || report.window.warmup_sessions != 200
        || report.window.evaluation_sessions != 2
        || !is_sha256_prefixed(&report.result_hashes.illustrative_10bp_per_side)
        || !is_sha256_prefixed(&report.result_hashes.zero_cost_reference)
    {
        return Err(Failure::ControlFile);
    }
    let checks = &report.checks;
    if !checks.sealed_admission
        || !checks.authenticated_enqueue
        || !checks.pending_pair_hidden
        || !checks.atomic_success
        || !checks.whole_pair_binding
        || !checks.foreign_api_denied
        || !checks.foreign_rls_denied
        || !checks.durable_replay
        || !checks.rights_revocation
        || !checks.scratch_database_absent
        || !checks.scratch_artifact_dir_absent
        || report.counts.jobs != 1
        || report.counts.attempts != 1
        || report.counts.pairs != 1
    {
        return Err(Failure::ControlFile);
    }
    let mut bytes = serde_json::to_vec(report).map_err(|_| Failure::ControlFile)?;
    bytes.push(b'\n');
    write_private_new(&control_dir.join(REPORT_NAME), control_dir, &bytes)
}

fn write_private_new(path: &Path, parent: &Path, bytes: &[u8]) -> ProofResult<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let mut file = options.open(path).map_err(|_| Failure::ControlFile)?;
        let write_result = (|| {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Failure::ControlFile)?;
            file.write_all(bytes).map_err(|_| Failure::ControlFile)?;
            file.sync_all().map_err(|_| Failure::ControlFile)?;
            let metadata = file.metadata().map_err(|_| Failure::ControlFile)?;
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o7777 != 0o600
            {
                return Err(Failure::ControlFile);
            }
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| Failure::ControlFile)?;
            Ok(())
        })();
        drop(file);
        if write_result.is_err() {
            let _ = fs::remove_file(path);
        }
        write_result
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, parent, bytes);
        Err(Failure::ControlFile)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use job_queue::owner_beta::simulation::{
        CostProfile, EvaluationPriceRow, Price4, SignalSnapshot, SimulationResult,
        StrategyIdentity, simulate_ledgers,
    };
    use std::{ffi::OsString, fs, path::Path};

    fn setup_cli_dirs(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let artifact_root = root.join(ROOT_BASENAME);
        let delivery_dir = root.join("delivery");
        let control_dir = root.join("control");
        fs::create_dir(&artifact_root).expect("synthetic artifact root");
        fs::create_dir(&delivery_dir).expect("synthetic delivery directory");
        fs::create_dir(&control_dir).expect("synthetic control directory");
        fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o700))
            .expect("private synthetic control directory");
        (artifact_root, delivery_dir, control_dir)
    }

    fn cli_arguments(
        artifact_root: &Path,
        delivery_dir: &Path,
        control_dir: &Path,
        proof_hash: &str,
        runner_hash: &str,
        nonce: &str,
    ) -> Vec<OsString> {
        [
            "--approved-connection-proof".to_owned(),
            "--artifact-root".to_owned(),
            artifact_root.to_string_lossy().into_owned(),
            "--delivery-dir".to_owned(),
            delivery_dir.to_string_lossy().into_owned(),
            "--control-dir".to_owned(),
            control_dir.to_string_lossy().into_owned(),
            "--proof-receipt-sha256".to_owned(),
            proof_hash.to_owned(),
            "--runner-receipt-sha256".to_owned(),
            runner_hash.to_owned(),
            "--scratch-nonce".to_owned(),
            nonce.to_owned(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect()
    }

    fn synthetic_origin() -> BuildOrigin {
        BuildOrigin {
            git_commit: "a".repeat(40),
            toolchain: "rustc-1.97.1".to_owned(),
            target: "x86_64-unknown-linux-musl".to_owned(),
            profile: "release".to_owned(),
            workflow_run_id: "123456789".to_owned(),
            workflow_run_attempt: "2".to_owned(),
        }
    }

    fn synthetic_receipt(schema: &str, origin: &BuildOrigin, binary: &[u8]) -> BuildReceipt {
        BuildReceipt {
            schema: schema.to_owned(),
            git_commit: origin.git_commit.clone(),
            toolchain: origin.toolchain.clone(),
            target: origin.target.clone(),
            profile: origin.profile.clone(),
            workflow_run_id: origin.workflow_run_id.clone(),
            workflow_run_attempt: origin.workflow_run_attempt.clone(),
            binary_sha256: sha256_hex(binary),
            source_hashes: BTreeMap::from([(
                "crates/api-server/src/lib.rs".to_owned(),
                "b".repeat(64),
            )]),
        }
    }

    fn synthetic_engine(commit: char, binary: char) -> EngineIdentity {
        EngineIdentity::new(
            &commit.to_string().repeat(40),
            &format!("sha256:{}", binary.to_string().repeat(64)),
            "rustc-test",
            "x86_64-unknown-linux-gnu",
            "test",
        )
        .expect("synthetic typed engine identity")
    }

    fn synthetic_result_bytes(
        input: &SimulationJobInput,
        engine: EngineIdentity,
        profile: CostProfile,
    ) -> Vec<u8> {
        let schedule = input.schedule();
        let factor_hash = format!("sha256:{}", "b".repeat(64));
        let target_hash = format!("sha256:{}", "c".repeat(64));
        let mut targets = market_data::KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| {
                let instrument = format!("{symbol}.KRX");
                let weight = if instrument == "069500.KRX" {
                    1_000_000
                } else {
                    0
                };
                (instrument, weight)
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.0.cmp(&right.0));
        let signals = schedule
            .signal_execution_pairs()
            .into_iter()
            .map(|(signal_date, _)| {
                SignalSnapshot::new(
                    &signal_date.to_iso(),
                    &factor_hash,
                    &target_hash,
                    targets.clone(),
                    0,
                )
                .expect("synthetic checked signal")
            })
            .collect::<Vec<_>>();
        let open = Price4::parse("100.0000").expect("synthetic open price");
        let close = Price4::parse("101.0000").expect("synthetic close price");
        let prices = schedule
            .evaluation_sessions()
            .iter()
            .map(|session| {
                EvaluationPriceRow::new(&session.to_iso(), open.clone(), close.clone(), 1_000_000)
                    .expect("synthetic checked price")
            })
            .collect::<Vec<_>>();
        let ledgers =
            simulate_ledgers(schedule, &signals, &prices).expect("synthetic typed ledgers");
        let strategy = StrategyIdentity::trend_following_v1(
            input.strategy().config_sha256().as_str(),
            "1.0.0",
            &format!("sha256:{}", "d".repeat(64)),
        )
        .expect("synthetic fixed strategy identity");
        let ledgers = match profile {
            CostProfile::Illustrative10BpPerSideV1 => ledgers.illustrative_10bp_per_side(),
            CostProfile::ZeroCostReferenceV1 => ledgers.zero_cost_reference(),
        };
        SimulationResult::new(
            engine,
            strategy,
            schedule.clone(),
            profile,
            signals,
            ledgers.strategy().clone(),
            ledgers.benchmark().clone(),
        )
        .expect("synthetic result contract")
        .canonical_json()
        .expect("synthetic canonical result")
    }

    #[test]
    fn proof_cli_rejects_missing_duplicate_unknown_and_nonisolated_options() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temp = tempfile::tempdir().expect("synthetic temp directory");
        let (root, delivery, control) = setup_cli_dirs(temp.path());
        let proof_hash = "a".repeat(64);
        let runner_hash = "b".repeat(64);
        let nonce = "c".repeat(32);
        let valid = cli_arguments(
            &root,
            &delivery,
            &control,
            &proof_hash,
            &runner_hash,
            &nonce,
        );
        assert!(parse_arguments(valid.clone(), true).is_ok());

        assert!(parse_arguments([OsString::from("--approved-connection-proof")], true).is_err());
        let mut duplicate = valid.clone();
        duplicate.extend([
            OsString::from("--artifact-root"),
            root.as_os_str().to_owned(),
        ]);
        assert!(parse_arguments(duplicate, true).is_err());
        let mut unknown = valid.clone();
        unknown.extend([OsString::from("--window"), OsString::from("2017-06-22")]);
        assert!(parse_arguments(unknown, true).is_err());
        assert!(parse_arguments(valid.clone(), false).is_err());

        let mut relative = valid.clone();
        relative[2] = OsString::from(ROOT_BASENAME);
        assert!(parse_arguments(relative, true).is_err());
        let wrong_root = temp.path().join("another-root");
        fs::create_dir(&wrong_root).expect("synthetic wrong root");
        let wrong_name = cli_arguments(
            &wrong_root,
            &delivery,
            &control,
            &proof_hash,
            &runner_hash,
            &nonce,
        );
        assert!(parse_arguments(wrong_name, true).is_err());

        let delivery_link = temp.path().join("delivery-link");
        symlink(&delivery, &delivery_link).expect("synthetic symlink");
        let linked = cli_arguments(
            &root,
            &delivery_link,
            &control,
            &proof_hash,
            &runner_hash,
            &nonce,
        );
        assert!(parse_arguments(linked, true).is_err());

        let mut uppercase_hash = valid.clone();
        uppercase_hash[8] = OsString::from("A".repeat(64));
        assert!(parse_arguments(uppercase_hash, true).is_err());
        let mut malformed_nonce = valid.clone();
        malformed_nonce[12] = OsString::from("c".repeat(31));
        assert!(parse_arguments(malformed_nonce, true).is_err());
        let mut uppercase_nonce = valid.clone();
        uppercase_nonce[12] = OsString::from("C".repeat(32));
        assert!(parse_arguments(uppercase_nonce, true).is_err());

        fs::set_permissions(&control, fs::Permissions::from_mode(0o755))
            .expect("make control directory nonprivate");
        assert!(parse_arguments(valid.clone(), true).is_err());
        let current_uid = unsafe { libc::geteuid() };
        assert!(!control_metadata_matches(
            current_uid.wrapping_add(1),
            0o700,
            current_uid
        ));
        fs::set_permissions(&control, fs::Permissions::from_mode(0o700))
            .expect("restore private control directory");

        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStringExt;
            let mut non_utf8 = valid;
            non_utf8[12] = OsString::from_vec(vec![0xff]);
            assert!(parse_arguments(non_utf8, true).is_err());
        }
    }

    #[test]
    fn proof_receipts_bind_origin_compile_and_binaries() {
        let origin = synthetic_origin();
        assert_eq!(
            compile_origin(CompileConstants {
                git_commit: Some(&origin.git_commit),
                toolchain: Some(&origin.toolchain),
                target: Some(&origin.target),
                profile: Some(&origin.profile),
                workflow_run_id: Some(&origin.workflow_run_id),
                workflow_run_attempt: Some(&origin.workflow_run_attempt),
            })
            .expect("synthetic compile constants validate"),
            origin
        );
        let proof_binary = b"proof executable bytes";
        let runner_binary = b"runner executable bytes";
        let proof = synthetic_receipt(PROOF_SCHEMA, &origin, proof_binary);
        let runner = synthetic_receipt(RUNNER_SCHEMA, &origin, runner_binary);
        assert!(proof.validate(PROOF_SCHEMA, &origin, &sha256_hex(proof_binary)));
        assert!(runner.validate(RUNNER_SCHEMA, &origin, &sha256_hex(runner_binary)));

        assert!(
            compile_origin(CompileConstants {
                git_commit: None,
                toolchain: None,
                target: None,
                profile: None,
                workflow_run_id: None,
                workflow_run_attempt: None,
            })
            .is_err()
        );

        let proof_bytes = serde_json::to_vec(&proof).expect("synthetic receipt serializes");
        assert_eq!(
            parse_receipt(&proof_bytes).expect("strict receipt parses"),
            proof
        );

        let temp = tempfile::tempdir().expect("synthetic receipt directory");
        let artifact_root = temp.path().join(ROOT_BASENAME);
        let delivery_dir = temp.path().join("delivery");
        let control_dir = temp.path().join("control");
        fs::create_dir(&artifact_root).expect("synthetic artifact root");
        fs::create_dir(&delivery_dir).expect("synthetic delivery directory");
        fs::create_dir(&control_dir).expect("synthetic control directory");
        let proof_path = delivery_dir.join(PROOF_RECEIPT_NAME);
        let runner_receipt_path = delivery_dir.join(RUNNER_RECEIPT_NAME);
        let runner_path = delivery_dir.join(RUNNER_NAME);
        let mut file_bound_proof = synthetic_receipt(PROOF_SCHEMA, &origin, b"unused");
        file_bound_proof.binary_sha256 =
            hash_current_executable().expect("synthetic test executable can be hashed");
        let file_bound_runner = synthetic_receipt(RUNNER_SCHEMA, &origin, runner_binary);
        let file_bound_proof_bytes =
            serde_json::to_vec(&file_bound_proof).expect("synthetic proof receipt serializes");
        let file_bound_runner_bytes =
            serde_json::to_vec(&file_bound_runner).expect("synthetic runner receipt serializes");
        fs::write(&proof_path, &file_bound_proof_bytes).expect("synthetic proof receipt file");
        fs::write(&runner_receipt_path, &file_bound_runner_bytes)
            .expect("synthetic runner receipt file");
        fs::write(&runner_path, runner_binary).expect("synthetic runner file");
        let options = Options {
            artifact_root,
            delivery_dir,
            control_dir,
            proof_receipt_sha256: sha256_hex(&file_bound_proof_bytes),
            runner_receipt_sha256: sha256_hex(&file_bound_runner_bytes),
            scratch_nonce: "c".repeat(32),
        };
        let constants = CompileConstants {
            git_commit: Some(&origin.git_commit),
            toolchain: Some(&origin.toolchain),
            target: Some(&origin.target),
            profile: Some(&origin.profile),
            workflow_run_id: Some(&origin.workflow_run_id),
            workflow_run_attempt: Some(&origin.workflow_run_attempt),
        };
        assert!(verify_provenance(&options, constants).is_ok());
        fs::write(&runner_path, b"changed runner bytes").expect("synthetic runner file changes");
        assert!(verify_provenance(&options, constants).is_err());

        let unknown = serde_json::json!({
            "schema": PROOF_SCHEMA,
            "git_commit": origin.git_commit.clone(),
            "toolchain": origin.toolchain.clone(),
            "target": origin.target.clone(),
            "profile": origin.profile.clone(),
            "workflow_run_id": origin.workflow_run_id.clone(),
            "workflow_run_attempt": origin.workflow_run_attempt.clone(),
            "binary_sha256": proof.binary_sha256.clone(),
            "source_hashes": proof.source_hashes.clone(),
            "unexpected": true
        });
        assert!(parse_receipt(&serde_json::to_vec(&unknown).expect("unknown field JSON")).is_err());
        assert!(parse_receipt(br#"{"schema":"first","schema":"second"}"#).is_err());

        let duplicate_nested = format!(
            "{{\"schema\":\"{PROOF_SCHEMA}\",\"source_hashes\":{{\"src/lib.rs\":\"{}\",\"src/lib.rs\":\"{}\"}}}}",
            "b".repeat(64),
            "c".repeat(64),
        );
        assert!(parse_receipt(duplicate_nested.as_bytes()).is_err());

        assert!(!proof.validate(PROOF_SCHEMA, &origin, &sha256_hex(runner_binary)));
        assert!(!proof.validate(RUNNER_SCHEMA, &origin, &sha256_hex(proof_binary)));
        let mut empty_sources = proof.clone();
        empty_sources.source_hashes.clear();
        assert!(!empty_sources.validate(PROOF_SCHEMA, &origin, &sha256_hex(proof_binary)));
        let mut malformed_source_hash = proof.clone();
        malformed_source_hash
            .source_hashes
            .insert("crates/api-server/src/lib.rs".to_owned(), "A".repeat(64));
        assert!(!malformed_source_hash.validate(PROOF_SCHEMA, &origin, &sha256_hex(proof_binary)));
        let mut wrong_origin = origin.clone();
        wrong_origin.workflow_run_attempt = "3".to_owned();
        assert!(!runner.validate(RUNNER_SCHEMA, &wrong_origin, &sha256_hex(runner_binary)));
        assert!(!is_lower_hex(&"A".repeat(64), 64));
        assert!(!positive_ascii_number("0"));
        assert!(!safe_relative_source_path("../src/lib.rs"));
    }

    #[test]
    fn proof_scratch_guard_requires_socket_nonce_and_empty_cluster() {
        let valid = ScratchObservation {
            current_database: "postgres".to_owned(),
            unix_socket: true,
            listen_addresses: String::new(),
            cluster_name: format!("r4-proof-{}", "a".repeat(32)),
            non_template_databases: vec!["postgres".to_owned()],
        };
        let nonce = "a".repeat(32);
        assert!(scratch_guard_accepts(DATABASE_URL, &nonce, &valid));
        assert!(!scratch_guard_accepts(
            "postgres://postgres@127.0.0.1/postgres?host=/r4-proof-db",
            &nonce,
            &valid
        ));
        assert!(!scratch_guard_accepts(
            "postgres://postgres@localhost/postgres?host=/r4-proof-db&sslmode=disable",
            &nonce,
            &valid
        ));

        let mut wrong_nonce = valid.clone();
        wrong_nonce.cluster_name = format!("r4-proof-{}", "b".repeat(32));
        assert!(!scratch_guard_accepts(DATABASE_URL, &nonce, &wrong_nonce));
        let mut tcp = valid.clone();
        tcp.unix_socket = false;
        assert!(!scratch_guard_accepts(DATABASE_URL, &nonce, &tcp));
        let mut listener = valid.clone();
        listener.listen_addresses = "127.0.0.1".to_owned();
        assert!(!scratch_guard_accepts(DATABASE_URL, &nonce, &listener));
        let mut populated = valid.clone();
        populated
            .non_template_databases
            .push("production".to_owned());
        assert!(!scratch_guard_accepts(DATABASE_URL, &nonce, &populated));
        let mut wrong_database = valid;
        wrong_database.current_database = "application".to_owned();
        assert!(!scratch_guard_accepts(
            DATABASE_URL,
            &nonce,
            &wrong_database
        ));
    }

    #[test]
    fn proof_control_files_are_private_closed_and_no_overwrite() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let temp = tempfile::tempdir().expect("synthetic temp directory");
        let control_dir = temp.path().join("control");
        fs::create_dir(&control_dir).expect("synthetic control directory");
        fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o700))
            .expect("private synthetic control directory");
        let run_id = Uuid::from_u128(10).to_string();
        let job_id = Uuid::from_u128(11).to_string();
        let ready = ReadyReceipt {
            schema: READY_SCHEMA.to_owned(),
            nonce: "c".repeat(32),
            db_name: "api24_scratch_1".to_owned(),
            run_id,
            job_id,
            worker_id: format!("r4-proof-{}", "c".repeat(32)),
        };
        write_ready(&control_dir, &ready).expect("private readiness receipt");
        let ready_path = control_dir.join(READY_NAME);
        let ready_metadata = fs::metadata(&ready_path).expect("readiness file metadata");
        assert_eq!(ready_metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(ready_metadata.uid(), unsafe { libc::geteuid() });
        let ready_bytes = fs::read(&ready_path).expect("read synthetic readiness receipt");
        assert_eq!(closed_json::<ReadyReceipt>(&ready_bytes).unwrap(), ready);
        assert!(write_ready(&control_dir, &ready).is_err());

        let mut unknown_ready: Value =
            serde_json::from_slice(&ready_bytes).expect("synthetic receipt JSON");
        unknown_ready["unexpected"] = Value::Bool(true);
        assert!(
            closed_json::<ReadyReceipt>(
                &serde_json::to_vec(&unknown_ready).expect("unknown readiness JSON")
            )
            .is_err()
        );
        assert!(strict_json_value(br#"{"schema":"one","schema":"two"}"#).is_err());

        let report = ProofReport {
            schema: REPORT_SCHEMA,
            status: "PASS",
            git_commit: "a".repeat(40),
            workflow_run_id: "123".to_owned(),
            workflow_run_attempt: "1".to_owned(),
            window: ProofWindow {
                start_date: START_DATE,
                end_date: END_DATE,
                warmup_sessions: 200,
                evaluation_sessions: 2,
            },
            result_hashes: ProofHashes {
                illustrative_10bp_per_side: format!("sha256:{}", "a".repeat(64)),
                zero_cost_reference: format!("sha256:{}", "b".repeat(64)),
            },
            checks: ProofChecks {
                sealed_admission: true,
                authenticated_enqueue: true,
                pending_pair_hidden: true,
                atomic_success: true,
                whole_pair_binding: true,
                foreign_api_denied: true,
                foreign_rls_denied: true,
                durable_replay: true,
                rights_revocation: true,
                scratch_database_absent: true,
                scratch_artifact_dir_absent: true,
            },
            counts: ProofCounts {
                jobs: 1,
                attempts: 1,
                pairs: 1,
            },
        };
        write_report(&control_dir, &report).expect("private proof report");
        assert!(write_report(&control_dir, &report).is_err());
        let report_path = control_dir.join(REPORT_NAME);
        let report_metadata = fs::metadata(&report_path).expect("proof report metadata");
        assert_eq!(report_metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(report_metadata.uid(), unsafe { libc::geteuid() });
        let report_value: Value =
            serde_json::from_slice(&fs::read(&report_path).expect("read synthetic proof report"))
                .expect("synthetic proof report JSON");
        let report_keys = report_value
            .as_object()
            .expect("closed report object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            report_keys,
            std::collections::BTreeSet::from([
                "schema".to_owned(),
                "status".to_owned(),
                "git_commit".to_owned(),
                "workflow_run_id".to_owned(),
                "workflow_run_attempt".to_owned(),
                "window".to_owned(),
                "result_hashes".to_owned(),
                "checks".to_owned(),
                "counts".to_owned(),
            ])
        );

        let rejected_dir = temp.path().join("rejected-control");
        fs::create_dir(&rejected_dir).expect("synthetic rejected report directory");
        fs::set_permissions(&rejected_dir, fs::Permissions::from_mode(0o700))
            .expect("private rejected report directory");
        let mut rejected = report;
        rejected.checks.rights_revocation = false;
        assert!(write_report(&rejected_dir, &rejected).is_err());
        assert!(!rejected_dir.join(REPORT_NAME).exists());
    }

    #[test]
    fn proof_pair_rejects_mixed_binding_and_truncated_content() {
        let input = Harness::simulation_test_input(
            Uuid::from_u128(20),
            Uuid::from_u128(21),
            START_DATE,
            END_DATE,
        );
        let job_id = Uuid::from_u128(22);
        // The API projects a claimed job as RUNNING before the simulation row
        // receives its start timestamp at publication/settlement. Parse that
        // production-shaped response, not a fixture with an invented start.
        let metadata_json = serde_json::json!({
            "id": input.run_id().to_string(),
            "job_id": job_id.to_string(),
            "strategy_config_id": input.strategy_config_id().to_string(),
            "strategy_id": "trend_following",
            "strategy_version": "1.0.0",
            "start_date": START_DATE,
            "end_date": END_DATE,
            "evaluation_session_count": 2,
            "status": "RUNNING",
            "audience": "OWNER_ONLY",
            "capability": "PRICE_RETURN_ONLY",
            "vendor_snapshot": true,
            "strict_pit": false,
            "contract_version": job_queue::owner_beta::simulation::CONTRACT_VERSION,
            "contract_document_sha256": input.contract_document_sha256().as_str(),
            "result_schema_version": job_queue::owner_beta::simulation::RESULT_SCHEMA_VERSION,
            "created_at": "2026-10-24T00:00:00Z",
            "started_at": null,
            "finished_at": null,
            "updated_at": "2026-10-24T00:00:00Z",
            "error_code": null
        });
        let mut metadata: RunMetadata = closed_json(
            &serde_json::to_vec(&metadata_json).expect("synthetic run metadata serializes"),
        )
        .expect("production-shaped metadata parses");
        let is_valid = |value: &RunMetadata| {
            valid_metadata(value, input.run_id(), job_id, input.strategy_config_id())
        };
        assert!(
            is_valid(&metadata),
            "a claimed job may have no run start yet"
        );
        metadata.started_at = Some("2026-10-24T00:00:01Z".to_owned());
        metadata.updated_at = "2026-10-24T00:00:01Z".to_owned();
        assert!(is_valid(&metadata));
        metadata.finished_at = Some("2026-10-24T00:00:02Z".to_owned());
        metadata.updated_at = "2026-10-24T00:00:02Z".to_owned();
        assert!(!is_valid(&metadata), "RUNNING cannot already be finished");
        metadata.status = "SUCCEEDED".to_owned();
        assert!(is_valid(&metadata));
        metadata.started_at = None;
        assert!(!is_valid(&metadata), "success still requires a start");
        metadata.finished_at = None;
        assert!(!is_valid(&metadata), "success still requires a finish");
        metadata.started_at = Some("2026-10-24T00:00:01Z".to_owned());
        assert!(!is_valid(&metadata), "a start alone is not success");
        metadata.started_at = None;
        metadata.status = "PENDING".to_owned();
        assert!(is_valid(&metadata));
        metadata.status = "RUNNING".to_owned();
        metadata.id = Uuid::from_u128(23).to_string();
        assert!(
            !is_valid(&metadata),
            "nullable timestamps do not relax identity"
        );
        metadata.id = input.run_id().to_string();
        metadata.status = "FAILED".to_owned();
        assert!(!is_valid(&metadata), "a failed run is not a waiting state");

        let engine_a = synthetic_engine('a', 'a');
        let engine_b = synthetic_engine('b', 'b');
        let illustrative = synthetic_result_bytes(
            &input,
            engine_a.clone(),
            CostProfile::Illustrative10BpPerSideV1,
        );
        let zero_cost =
            synthetic_result_bytes(&input, engine_a.clone(), CostProfile::ZeroCostReferenceV1);
        let mixed_zero_cost =
            synthetic_result_bytes(&input, engine_b, CostProfile::ZeroCostReferenceV1);
        assert!(
            CheckedSimulationPair::from_canonical(&illustrative, &zero_cost, &input, &engine_a,)
                .is_ok()
        );
        assert!(
            CheckedSimulationPair::from_canonical(
                &illustrative,
                &mixed_zero_cost,
                &input,
                &engine_a,
            )
            .is_err()
        );
        assert!(
            CheckedSimulationPair::from_canonical(
                &illustrative[..illustrative.len() / 2],
                &zero_cost,
                &input,
                &engine_a,
            )
            .is_err()
        );
    }

    #[test]
    fn proof_role_url_preserves_unix_socket_query() {
        assert_eq!(
            crate::common::conn_url(
                "postgres://postgres@localhost/postgres?host=/r4-proof-db",
                "worker",
                "api24_scratch",
            ),
            "postgres://worker@localhost/api24_scratch?host=/r4-proof-db",
        );
        assert_eq!(
            crate::common::conn_url(
                "postgres://postgres:secret@127.0.0.1:5432/postgres",
                "app",
                "api24_scratch",
            ),
            "postgres://app:secret@127.0.0.1:5432/api24_scratch",
        );
    }
}
