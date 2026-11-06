use serde_json::Value;
use uuid::Uuid;

use super::Verb;

pub(super) const MAX_WIRE_BYTES: usize = 65_536;
pub(super) const MAX_DEPTH: usize = 16;
pub(super) const MAX_PAGE_BYTES: usize = 262_144;
pub(super) const MAX_PAGES: usize = 512;
pub(super) const MAX_EXACT_BYTES: usize = 1_048_576;

const INTENT_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "predecessor_generation",
    "request_key",
    "purpose",
    "scope",
    "expected_epoch",
    "local_epoch_floor",
    "release_manifest_sha256",
    "artifact_sha256",
    "resume_target_id",
];
const CONFIGURE_KEYS: &[&str] = &["schema", "strategy_config_id", "account_name", "contract"];
const CONTRACT_KEYS: &[&str] = &[
    "contract_id",
    "contract_sha256",
    "decision_sha256",
    "cost_profile_id",
    "cost_profile_version",
];
const PROCESS_BINDING_KEYS: &[&str] = &[
    "schema",
    "owner_user_id",
    "account_id",
    "strategy_config_id",
    "consent_event_id",
    "consent_version",
    "control_epoch",
    "daily_cohort_sha256",
    "financial_contract_sha256",
    "code_commit",
    "release_manifest_sha256",
    "api_binary_sha256",
    "worker_binary_sha256",
];
const CAPABILITY_KEYS: &[&str] = &[
    "schema",
    "minimum",
    "code_commit",
    "release_manifest_sha256",
    "operations_schema_sha256",
    "archive_schema_sha256",
    "migration_0068_sha256",
    "audit_sql_sha256",
    "api_binary_sha256",
    "worker_binary_sha256",
    "native_binary_sha256",
];
const INVENTORY_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "scope",
    "fence_event_id",
    "control_epoch",
    "boot_id",
    "observed",
    "release_manifest_sha256",
    "writers",
    "launch_fence_sha256",
    "backend_observation_sha256",
];
const WRITER_KEYS: &[&str] = &[
    "writer_class",
    "kind",
    "identity_sha256",
    "observation_sha256",
    "backend_set_sha256",
    "terminal",
];
const CLOCK_KEYS: &[&str] = &["realtime_ns", "monotonic_ns", "boottime_ns"];
const DRAIN_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "request_key",
    "scope",
    "expected_epoch",
    "resulting_epoch",
    "intent_sha256",
    "stage_sha256",
    "inventory_sha256",
    "reconciliation_sha256",
    "observation_id",
    "release_manifest_sha256",
    "boot_id",
    "observed",
];
const RESTORE_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "request_key",
    "scope",
    "expected_epoch",
    "resulting_epoch",
    "intent_sha256",
    "stage_sha256",
    "inventory_sha256",
    "reconciliation_sha256",
    "observation_id",
    "release_manifest_sha256",
    "boot_id",
    "observed",
    "source_installation_id",
    "source_generation",
    "source_epoch",
    "archive_sha256",
    "source_audit_sha256",
    "restored_audit_sha256",
    "destination_epoch_floor",
];
const RECOVERY_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "request_key",
    "pending_target_id",
    "lineage_id",
    "effective_session",
    "lineage_epoch",
    "active_epoch",
    "original_receipt_sha256",
    "consent_event_id",
    "process_config_sha256",
    "step",
    "signal_admission_sha256",
    "execution_admission_sha256",
    "decision_sha256",
    "reason_code",
    "expires_at",
];

#[derive(Clone, Debug)]
pub(super) struct Intent {
    pub value: Value,
    pub installation_id: Uuid,
    pub generation: Uuid,
    pub account_id: Option<Uuid>,
    pub request_key: String,
    pub purpose: String,
    pub scope: String,
    pub expected_epoch: i64,
    pub local_epoch_floor: i64,
    pub release_manifest_sha256: String,
    pub artifact_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Artifact {
    pub value: Value,
    pub sha256: String,
}

pub(super) fn parse_canonical(bytes: &[u8]) -> Result<Value, ()> {
    parse_canonical_limit(bytes, MAX_WIRE_BYTES)
}

fn parse_canonical_limit(bytes: &[u8], limit: usize) -> Result<Value, ()> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    validate_tree(&value, 1)?;
    let canonical = super::super::source_evidence::canonical(&value).map_err(|_| ())?;
    if canonical != bytes {
        return Err(());
    }
    Ok(value)
}

fn validate_tree(value: &Value, depth: usize) -> Result<(), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if !is_clean_ascii(key) {
                    return Err(());
                }
                validate_tree(child, depth + 1)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                validate_tree(child, depth + 1)?;
            }
        }
        Value::String(text) if !is_clean_ascii(text) => return Err(()),
        Value::Number(number) if number.as_i64().is_none() => return Err(()),
        _ => {}
    }
    Ok(())
}

fn is_clean_ascii(text: &str) -> bool {
    text.is_ascii() && !text.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
}

fn exact_keys(value: &Value, expected: &[&str]) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(());
    }
    Ok(())
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, ()> {
    value.get(key).and_then(Value::as_str).ok_or(())
}

fn i64_value(value: &Value, key: &str) -> Result<i64, ()> {
    let number = value.get(key).and_then(Value::as_i64).ok_or(())?;
    if number < 0 {
        return Err(());
    }
    Ok(number)
}

fn optional_uuid(value: &Value, key: &str) -> Result<Option<Uuid>, ()> {
    match value.get(key).ok_or(())? {
        Value::Null => Ok(None),
        Value::String(value) => parse_uuid(value).map(Some),
        _ => Err(()),
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, ()> {
    let parsed = Uuid::parse_str(value).map_err(|_| ())?;
    if parsed.is_nil() || parsed.to_string() != value {
        return Err(());
    }
    Ok(parsed)
}

fn digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn parse_intent(bytes: &[u8], verb: Verb) -> Result<Intent, ()> {
    if verb == Verb::Readback {
        return Err(());
    }
    let verb = if verb == Verb::ObserveReset {
        Verb::Inspect
    } else {
        verb
    };
    let value = parse_canonical(bytes)?;
    exact_keys(&value, INTENT_KEYS)?;
    if text(&value, "schema")? != "owner-beta-paper-operation-intent-v1" {
        return Err(());
    }

    let installation_id = parse_uuid(text(&value, "installation_id")?)?;
    let generation = parse_uuid(text(&value, "generation")?)?;
    let account_id = optional_uuid(&value, "account_id")?;
    let predecessor_generation = optional_uuid(&value, "predecessor_generation")?;
    let resume_target_id = optional_uuid(&value, "resume_target_id")?;
    let request_key = text(&value, "request_key")?.to_owned();
    if request_key.is_empty()
        || request_key.len() > 128
        || !request_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err(());
    }
    let purpose = text(&value, "purpose")?.to_owned();
    let scope = text(&value, "scope")?.to_owned();
    let expected_epoch = i64_value(&value, "expected_epoch")?;
    let local_epoch_floor = i64_value(&value, "local_epoch_floor")?;
    let release_manifest_sha256 = text(&value, "release_manifest_sha256")?.to_owned();
    if !digest(&release_manifest_sha256) {
        return Err(());
    }
    let artifact_sha256 = match value.get("artifact_sha256").ok_or(())? {
        Value::Null => None,
        Value::String(hash) if digest(hash) => Some(hash.clone()),
        _ => return Err(()),
    };
    if predecessor_generation == Some(generation) || resume_target_id == Some(Uuid::nil()) {
        return Err(());
    }

    let expected_purpose = match verb {
        Verb::Configure => "CONFIGURE",
        Verb::Inspect => "INSPECT",
        Verb::Enable => "ENABLE",
        Verb::Recover => "RECOVERY",
        Verb::Begin | Verb::Reconcile | Verb::Off | Verb::RestoreOff => purpose.as_str(),
        Verb::Readback | Verb::ObserveReset => return Err(()),
    };
    if purpose != expected_purpose
        || ![
            "CONFIGURE",
            "INSPECT",
            "ENABLE",
            "STOP",
            "BACKUP",
            "RESTORE",
            "RECOVERY",
        ]
        .contains(&purpose.as_str())
    {
        return Err(());
    }
    if !["PAPER_STOP", "COMBINED_BACKUP"].contains(&scope.as_str()) {
        return Err(());
    }
    if verb == Verb::Configure {
        if expected_epoch != 0 || account_id.is_some() {
            return Err(());
        }
    } else if verb == Verb::Inspect {
        if account_id.is_none() && expected_epoch != 0 {
            return Err(());
        }
    } else if account_id.is_none() {
        return Err(());
    }
    if matches!(
        verb,
        Verb::Configure | Verb::Inspect | Verb::Enable | Verb::Recover
    ) && (local_epoch_floor != expected_epoch || predecessor_generation.is_some())
    {
        return Err(());
    }
    if matches!(
        verb,
        Verb::Begin | Verb::Reconcile | Verb::Off | Verb::RestoreOff
    ) && ((purpose == "RESTORE") != (scope == "COMBINED_BACKUP"))
    {
        return Err(());
    }
    if verb == Verb::RestoreOff && purpose != "RESTORE" {
        return Err(());
    }
    let generation_prefix = generation.to_string();
    let request_key_bound = match verb {
        Verb::Begin => request_key == format!("{generation_prefix}:begin"),
        Verb::Reconcile => request_key
            .strip_prefix(&format!("{generation_prefix}:reconcile:"))
            .is_some_and(|sequence| {
                !sequence.is_empty()
                    && sequence.len() <= 18
                    && sequence.bytes().all(|b| b.is_ascii_digit())
            }),
        Verb::Off => request_key == format!("{generation_prefix}:off"),
        Verb::RestoreOff => request_key == format!("{generation_prefix}:restore-off"),
        Verb::Recover => request_key == format!("{generation_prefix}:recover"),
        Verb::Enable => request_key == format!("{generation_prefix}:enable"),
        Verb::Configure | Verb::Inspect => true,
        Verb::Readback | Verb::ObserveReset => return Err(()),
    };
    if !request_key_bound {
        return Err(());
    }
    if purpose != "RESTORE" && local_epoch_floor != expected_epoch {
        return Err(());
    }
    Ok(Intent {
        value,
        installation_id,
        generation,
        account_id,
        request_key,
        purpose,
        scope,
        expected_epoch,
        local_epoch_floor,
        release_manifest_sha256,
        artifact_sha256,
    })
}

pub(super) fn parse_configure(bytes: &[u8]) -> Result<Value, ()> {
    let value = parse_canonical(bytes)?;
    exact_keys(&value, CONFIGURE_KEYS)?;
    if text(&value, "schema")? != "owner-beta-paper-configure-operation-v1"
        || parse_uuid(text(&value, "strategy_config_id")?).is_err()
        || text(&value, "account_name")? != "Owner-beta Paper"
    {
        return Err(());
    }
    let contract = value.get("contract").ok_or(())?;
    exact_keys(contract, CONTRACT_KEYS)?;
    if text(contract, "contract_id")? != "owner-beta-paper-v1-draft-1"
        || text(contract, "contract_sha256")?
            != "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0"
        || text(contract, "decision_sha256")?
            != "sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394"
        || text(contract, "cost_profile_id")? != "KRX_ETF_DEFAULT"
        || contract.get("cost_profile_version").and_then(Value::as_i64) != Some(1)
    {
        return Err(());
    }
    Ok(value)
}

pub(super) fn parse_process_binding(bytes: &[u8], intent: &Intent) -> Result<Artifact, ()> {
    let value = parse_canonical(bytes)?;
    exact_keys(&value, PROCESS_BINDING_KEYS)?;
    if text(&value, "schema")? != "owner-beta-paper-process-v1"
        || text(&value, "consent_version")? != "owner-beta-paper-v1-draft-1"
        || text(&value, "financial_contract_sha256")?
            != "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0"
        || text(&value, "release_manifest_sha256")? != intent.release_manifest_sha256
        || i64_value(&value, "control_epoch")? != intent.expected_epoch.checked_add(1).ok_or(())?
        || text(&value, "code_commit")?.len() != 40
        || !text(&value, "code_commit")?
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(());
    }
    for key in [
        "owner_user_id",
        "account_id",
        "strategy_config_id",
        "consent_event_id",
    ] {
        parse_uuid(text(&value, key)?)?;
    }
    if Some(parse_uuid(text(&value, "account_id")?)?) != intent.account_id {
        return Err(());
    }
    for key in [
        "daily_cohort_sha256",
        "api_binary_sha256",
        "worker_binary_sha256",
    ] {
        if !digest(text(&value, key)?) {
            return Err(());
        }
    }
    let sha256 = super::super::source_evidence::digest(bytes);
    if intent.artifact_sha256.as_deref() != Some(sha256.as_str()) {
        return Err(());
    }
    Ok(Artifact { value, sha256 })
}

pub(super) fn parse_capability(bytes: &[u8]) -> Result<Value, ()> {
    let value = parse_canonical(bytes)?;
    exact_keys(&value, CAPABILITY_KEYS)?;
    if text(&value, "schema")? != "owner-beta-paper-reader-capability-v1"
        || i64_value(&value, "minimum")? != 1
        || text(&value, "code_commit")?.len() != 40
        || !text(&value, "code_commit")?
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(());
    }
    for key in [
        "release_manifest_sha256",
        "operations_schema_sha256",
        "archive_schema_sha256",
        "migration_0068_sha256",
        "audit_sql_sha256",
        "api_binary_sha256",
        "worker_binary_sha256",
        "native_binary_sha256",
    ] {
        if !digest(text(&value, key)?) {
            return Err(());
        }
    }
    Ok(value)
}

fn validate_clock(value: &Value) -> Result<(), ()> {
    exact_keys(value, CLOCK_KEYS)?;
    for key in CLOCK_KEYS {
        i64_value(value, key)?;
    }
    Ok(())
}

pub(super) fn parse_artifact(
    bytes: &[u8],
    expected_schema: &str,
    intent: &Intent,
) -> Result<Artifact, ()> {
    let value = parse_canonical(bytes)?;
    match expected_schema {
        "owner-beta-paper-writer-inventory-v1" => {
            exact_keys(&value, INVENTORY_KEYS)?;
            if text(&value, "schema")? != expected_schema
                || text(&value, "installation_id")? != intent.installation_id.to_string()
                || text(&value, "generation")? != intent.generation.to_string()
                || Some(parse_uuid(text(&value, "account_id")?)?) != intent.account_id
                || text(&value, "scope")? != intent.scope
                || text(&value, "release_manifest_sha256")? != intent.release_manifest_sha256
                || parse_uuid(text(&value, "fence_event_id")?).is_err()
                || parse_uuid(text(&value, "boot_id")?).is_err()
                || !digest(text(&value, "launch_fence_sha256")?)
                || !digest(text(&value, "backend_observation_sha256")?)
            {
                return Err(());
            }
            i64_value(&value, "control_epoch")?;
            validate_clock(value.get("observed").ok_or(())?)?;
            let writers = value.get("writers").and_then(Value::as_array).ok_or(())?;
            if writers.len() > 64 {
                return Err(());
            }
            let mut previous_writer: Option<(String, String, String)> = None;
            let mut unique_writers = std::collections::BTreeSet::new();
            for writer in writers {
                exact_keys(writer, WRITER_KEYS)?;
                let writer_class = text(writer, "writer_class")?;
                let writer_kind = text(writer, "kind")?;
                let identity_sha256 = text(writer, "identity_sha256")?;
                if ![
                    "API",
                    "PAPER_RUNNER",
                    "PAPER_DELIVERY",
                    "SOURCE_PUBLISHER",
                    "SOURCE_CHILD",
                    "DAILY",
                    "ADMITTER",
                    "RECOMMENDATION",
                    "SIMULATION",
                    "RAW_CURATED",
                    "OTHER_DB_WRITER",
                ]
                .contains(&writer_class)
                    || !["CONTAINER", "PROCESS", "UNIT"].contains(&writer_kind)
                    || !digest(identity_sha256)
                    || !digest(text(writer, "observation_sha256")?)
                    || !digest(text(writer, "backend_set_sha256")?)
                    || !["EXITED_REAPED", "ABSENT_AT_FENCE"].contains(&text(writer, "terminal")?)
                {
                    return Err(());
                }
                let sort_key = (
                    identity_sha256.to_owned(),
                    writer_class.to_owned(),
                    writer_kind.to_owned(),
                );
                if previous_writer
                    .as_ref()
                    .is_some_and(|previous| previous >= &sort_key)
                    || !unique_writers.insert(sort_key.clone())
                {
                    return Err(());
                }
                previous_writer = Some(sort_key);
            }
        }
        "owner-beta-paper-drain-receipt-v1" => {
            exact_keys(&value, DRAIN_KEYS)?;
            if text(&value, "schema")? != expected_schema {
                return Err(());
            }
            validate_identity(&value, intent)?;
            validate_clock(value.get("observed").ok_or(())?)?;
        }
        "owner-beta-paper-restore-receipt-v1" => {
            exact_keys(&value, RESTORE_KEYS)?;
            if text(&value, "schema")? != expected_schema {
                return Err(());
            }
            validate_identity(&value, intent)?;
            validate_clock(value.get("observed").ok_or(())?)?;
            parse_uuid(text(&value, "source_installation_id")?)?;
            let source_generation = parse_uuid(text(&value, "source_generation")?)?;
            let source_epoch = i64_value(&value, "source_epoch")?;
            let destination_floor = i64_value(&value, "destination_epoch_floor")?;
            if Some(source_generation) != optional_uuid(&intent.value, "predecessor_generation")?
                || destination_floor != intent.local_epoch_floor
                || source_epoch > destination_floor
                || !digest(text(&value, "archive_sha256")?)
                || !digest(text(&value, "source_audit_sha256")?)
                || text(&value, "source_audit_sha256")? != text(&value, "restored_audit_sha256")?
            {
                return Err(());
            }
        }
        "owner-beta-paper-recovery-v2" => {
            exact_keys(&value, RECOVERY_KEYS)?;
            if text(&value, "schema")? != expected_schema {
                return Err(());
            }
            for key in [
                "installation_id",
                "generation",
                "account_id",
                "pending_target_id",
                "lineage_id",
                "consent_event_id",
            ] {
                parse_uuid(text(&value, key)?)?;
            }
            for key in [
                "original_receipt_sha256",
                "process_config_sha256",
                "signal_admission_sha256",
                "execution_admission_sha256",
                "decision_sha256",
            ] {
                if !digest(text(&value, key)?) {
                    return Err(());
                }
            }
            if text(&value, "installation_id")? != intent.installation_id.to_string()
                || text(&value, "generation")? != intent.generation.to_string()
                || Some(parse_uuid(text(&value, "account_id")?)?) != intent.account_id
                || text(&value, "request_key")? != intent.request_key
                || Some(parse_uuid(text(&value, "pending_target_id")?)?)
                    != optional_uuid(&intent.value, "resume_target_id")?
                || text(&value, "reason_code")? != "EXPLICIT_RETAINED_RECOVERY"
                || intent.request_key != format!("{}:recover", intent.generation)
            {
                return Err(());
            }
            let lineage_epoch = i64_value(&value, "lineage_epoch")?;
            let active_epoch = i64_value(&value, "active_epoch")?;
            if active_epoch != intent.expected_epoch || lineage_epoch >= active_epoch {
                return Err(());
            }
            if !["SETTLE_THEN_VALUE", "VALUE_ONLY"].contains(&text(&value, "step")?) {
                return Err(());
            }
            let session = text(&value, "effective_session")?;
            if session.len() != 10
                || session.as_bytes()[4] != b'-'
                || session.as_bytes()[7] != b'-'
                || !session
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
            {
                return Err(());
            }
            if chrono::NaiveDate::parse_from_str(session, "%Y-%m-%d").is_err() {
                return Err(());
            }
            let expires_at = text(&value, "expires_at")?;
            if expires_at.len() != 20
                || !expires_at.starts_with("20")
                || expires_at.as_bytes()[10] != b'T'
                || expires_at.as_bytes()[19] != b'Z'
                || !expires_at
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19) || b.is_ascii_digit())
            {
                return Err(());
            }
            if chrono::NaiveDateTime::parse_from_str(expires_at, "%Y-%m-%dT%H:%M:%SZ").is_err() {
                return Err(());
            }
        }
        _ => return Err(()),
    }
    let sha256 = super::super::source_evidence::digest(bytes);
    if intent.artifact_sha256.as_deref() != Some(sha256.as_str()) {
        return Err(());
    }
    Ok(Artifact { value, sha256 })
}

fn validate_identity(value: &Value, intent: &Intent) -> Result<(), ()> {
    if text(value, "installation_id")? != intent.installation_id.to_string()
        || text(value, "generation")? != intent.generation.to_string()
        || Some(parse_uuid(text(value, "account_id")?)?) != intent.account_id
        || text(value, "request_key")? != intent.request_key
        || text(value, "scope")? != intent.scope
        || text(value, "release_manifest_sha256")? != intent.release_manifest_sha256
        || !digest(text(value, "intent_sha256")?)
        || !digest(text(value, "stage_sha256")?)
        || !digest(text(value, "inventory_sha256")?)
        || !digest(text(value, "reconciliation_sha256")?)
    {
        return Err(());
    }
    if parse_uuid(text(value, "observation_id")?).is_err()
        || parse_uuid(text(value, "boot_id")?).is_err()
    {
        return Err(());
    }
    let intent_bytes = super::super::source_evidence::canonical(&intent.value).map_err(|_| ())?;
    if text(value, "intent_sha256")? != super::super::source_evidence::digest(&intent_bytes) {
        return Err(());
    }
    let expected_epoch = i64_value(value, "expected_epoch")?;
    let resulting_epoch = i64_value(value, "resulting_epoch")?;
    if expected_epoch != intent.expected_epoch
        || resulting_epoch != expected_epoch.checked_add(1).ok_or(())?
    {
        return Err(());
    }
    Ok(())
}

// These are closed untrusted request/result data, never permits. Original ASCII
// bytes (including embedded fractional timestamps) remain owned and unchanged.
pub(super) struct ExactRequest {
    pub value: Value,
    pub canonical: String,
    pub sha256: String,
}

impl ExactRequest {
    pub fn from_value(value: Value) -> Result<Self, ()> {
        let bytes = super::super::source_evidence::canonical(&value).map_err(|_| ())?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let value = parse_canonical_limit(bytes, MAX_EXACT_BYTES)?;
        exact_keys(&value, &["schema", "kind", "account_id", "args"])?;
        if text(&value, "schema")? != "owner-beta-paper-exact-request-v1" {
            return Err(());
        }
        let kind = text(&value, "kind")?;
        let account = optional_uuid(&value, "account_id")?;
        let args = &value["args"];
        if (kind == "CONFIGURE") != account.is_none() {
            return Err(());
        }
        let keys: &[&str] = match kind {
            "CONFIGURE" => &[
                "strategy_config_id",
                "account_name",
                "contract_canonical",
                "process_binding",
                "request_key",
            ],
            "BEGIN" => &["expected_epoch", "intent_canonical", "request_key"],
            "RECONCILE" => &[
                "generation",
                "expected_epoch",
                "inventory_sha256",
                "request_key",
            ],
            "TRANSITION" => &[
                "expected_epoch",
                "transition",
                "evidence_canonical",
                "request_key",
            ],
            "RESET" => &["expected_epoch"],
            _ => return Err(()),
        };
        exact_keys(args, keys)?;
        if kind != "CONFIGURE" {
            i64_value(args, "expected_epoch")?;
        }
        if kind != "RESET" {
            request_key(text(args, "request_key")?)?;
        }
        match kind {
            "CONFIGURE" => {
                parse_uuid(text(args, "strategy_config_id")?)?;
                let name = text(args, "account_name")?;
                if name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b" _.-".contains(&b))
                    || !args["process_binding"].is_null()
                {
                    return Err(());
                }
                let contract = embedded(args, "contract_canonical", MAX_PAGE_BYTES)?;
                validate_contract(&contract)?;
            }
            "BEGIN" => {
                let intent = parse_intent(text(args, "intent_canonical")?.as_bytes(), Verb::Begin)?;
                if !["STOP", "BACKUP", "RESTORE"].contains(&intent.purpose.as_str())
                    || intent.account_id != account
                    || intent.expected_epoch != i64_value(args, "expected_epoch")?
                    || intent.request_key != text(args, "request_key")?
                {
                    return Err(());
                }
            }
            "RECONCILE" => {
                let generation = parse_uuid(text(args, "generation")?)?;
                if !digest(text(args, "inventory_sha256")?)
                    || !reconcile_key(text(args, "request_key")?, generation)
                {
                    return Err(());
                }
            }
            "TRANSITION" => {
                let evidence = embedded(args, "evidence_canonical", MAX_PAGE_BYTES)?;
                validate_transition(text(args, "transition")?, &evidence)?;
            }
            "RESET" => {}
            _ => return Err(()),
        }
        Ok(Self {
            canonical: String::from_utf8(bytes.to_vec()).map_err(|_| ())?,
            sha256: super::super::source_evidence::digest(bytes),
            value,
        })
    }

    pub fn reset(account_id: Uuid, expected_epoch: i64) -> Result<Self, ()> {
        Self::from_value(serde_json::json!({
            "schema":"owner-beta-paper-exact-request-v1", "kind":"RESET",
            "account_id":account_id, "args":{"expected_epoch":expected_epoch}
        }))
    }

    pub fn original_verb(&self) -> Result<Verb, ()> {
        match text(&self.value, "kind")? {
            "CONFIGURE" => Ok(Verb::Configure),
            "BEGIN" => Ok(Verb::Begin),
            "RECONCILE" => Ok(Verb::Reconcile),
            "TRANSITION" => match text(&self.value["args"], "transition")? {
                "ENABLE" => Ok(Verb::Enable),
                "OFF" => Ok(Verb::Off),
                "RESTORED_OFF" => Ok(Verb::RestoreOff),
                "RECOVERY" => Ok(Verb::Recover),
                // DRAIN belongs to BEGIN's public operation identity. The old
                // native ABI has no DRAIN verb; do not invent a reconstruction.
                _ => Err(()),
            },
            _ => Err(()),
        }
    }

    pub fn correlate(&self, intent: &Intent, artifact: Option<&Artifact>) -> Result<(), ()> {
        let kind = text(&self.value, "kind")?;
        let args = &self.value["args"];
        if optional_uuid(&self.value, "account_id")? != intent.account_id
            || text(args, "request_key")? != intent.request_key
            || (kind != "CONFIGURE" && i64_value(args, "expected_epoch")? != intent.expected_epoch)
        {
            return Err(());
        }
        match kind {
            "CONFIGURE" => {
                let artifact = artifact.ok_or(())?;
                if args["strategy_config_id"] != artifact.value["strategy_config_id"]
                    || args["account_name"] != artifact.value["account_name"]
                    || embedded(args, "contract_canonical", MAX_PAGE_BYTES)?
                        != artifact.value["contract"]
                {
                    return Err(());
                }
            }
            "BEGIN" => {
                if embedded(args, "intent_canonical", MAX_WIRE_BYTES)? != intent.value {
                    return Err(());
                }
            }
            "RECONCILE" => {
                if text(args, "generation")? != intent.generation.to_string()
                    || text(args, "inventory_sha256")? != artifact.ok_or(())?.sha256
                {
                    return Err(());
                }
            }
            "TRANSITION" => {
                let evidence = embedded(args, "evidence_canonical", MAX_PAGE_BYTES)?;
                let artifact = artifact.ok_or(())?;
                if text(args, "transition")? == "ENABLE" {
                    let cohort = &evidence["current_cohort"];
                    if evidence["process_binding"] != artifact.value
                        || evidence["process_binding_sha256"] != artifact.sha256
                        || evidence["expected_epoch"].as_i64() != Some(intent.expected_epoch)
                        || evidence["new_epoch"].as_i64() != intent.expected_epoch.checked_add(1)
                        || cohort["release_manifest_sha256"] != intent.release_manifest_sha256
                        || cohort["cohort_sha256"] != artifact.value["daily_cohort_sha256"]
                        || cohort["owner_user_id"] != artifact.value["owner_user_id"]
                        || cohort["strategy_config_id"] != artifact.value["strategy_config_id"]
                    {
                        return Err(());
                    }
                } else if evidence != artifact.value {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

fn request_key(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        return Err(());
    }
    Ok(())
}

fn reconcile_key(key: &str, generation: Uuid) -> bool {
    key.strip_prefix(&format!("{generation}:reconcile:"))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.len() <= 18 && suffix.bytes().all(|b| b.is_ascii_digit())
        })
}

fn embedded(value: &Value, key: &str, limit: usize) -> Result<Value, ()> {
    parse_canonical_limit(text(value, key)?.as_bytes(), limit)
}

fn validate_contract(value: &Value) -> Result<(), ()> {
    // Reuse the existing closed SQL-v1 contract validator; no authority is
    // constructed, and the configure artifact still has its smaller wire limit.
    let bytes = super::super::source_evidence::canonical(&serde_json::json!({
        "schema":"owner-beta-paper-configure-operation-v1",
        "strategy_config_id":"11111111-1111-4111-8111-111111111111",
        "account_name":"Owner-beta Paper", "contract":value
    }))
    .map_err(|_| ())?;
    parse_configure(&bytes).map(|_| ())
}

const COHORT_KEYS: &[&str] = &[
    "owner_user_id",
    "policy_sha256",
    "activation_epoch",
    "strategy_config_id",
    "config_epoch",
    "strategy_config_sha256",
    "entitlement_reference",
    "entitlement_sha256",
    "cohort_sha256",
    "release_manifest_sha256",
    "research_image_id",
    "helper_image_id",
    "semantic_config_sha256",
    "calendar_sha256",
    "verifier_revision",
];

fn validate_cohort(value: &Value) -> Result<(), ()> {
    exact_keys(value, COHORT_KEYS)?;
    parse_uuid(text(value, "owner_user_id")?)?;
    parse_uuid(text(value, "strategy_config_id")?)?;
    for key in ["activation_epoch", "config_epoch"] {
        i64_value(value, key)?;
    }
    let reference = text(value, "entitlement_reference")?;
    if reference.is_empty() || reference.len() > 512 || !commit(text(value, "verifier_revision")?) {
        return Err(());
    }
    for key in COHORT_KEYS
        .iter()
        .filter(|k| k.ends_with("_sha256") || k.ends_with("_image_id"))
    {
        if !digest(text(value, key)?) {
            return Err(());
        }
    }
    Ok(())
}

fn commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_process(value: &Value) -> Result<(), ()> {
    exact_keys(value, PROCESS_BINDING_KEYS)?;
    if text(value, "schema")? != "owner-beta-paper-process-v1"
        || text(value, "consent_version")? != "owner-beta-paper-v1-draft-1"
        || text(value, "financial_contract_sha256")?
            != "sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0"
        || !commit(text(value, "code_commit")?)
    {
        return Err(());
    }
    for key in [
        "owner_user_id",
        "account_id",
        "strategy_config_id",
        "consent_event_id",
    ] {
        parse_uuid(text(value, key)?)?;
    }
    i64_value(value, "control_epoch")?;
    for key in PROCESS_BINDING_KEYS
        .iter()
        .filter(|k| k.ends_with("_sha256"))
    {
        if !digest(text(value, key)?) {
            return Err(());
        }
    }
    Ok(())
}

pub(super) fn validate_sql_sessions(value: &Value) -> Result<(), ()> {
    let sessions = value.as_array().ok_or(())?;
    if sessions.len() != 3 {
        return Err(());
    }
    let mut previous = None;
    let mut ids = std::collections::BTreeSet::new();
    for session in sessions {
        exact_keys(
            session,
            &[
                "session_date",
                "run_id",
                "admission_sha256",
                "source_event_sha256",
                "root_event_sha256",
            ],
        )?;
        let date = date(text(session, "session_date")?)?;
        if previous.is_some_and(|p| p >= date) {
            return Err(());
        }
        previous = Some(date);
        if !ids.insert(parse_uuid(text(session, "run_id")?)?) {
            return Err(());
        }
        for key in [
            "admission_sha256",
            "source_event_sha256",
            "root_event_sha256",
        ] {
            if !digest(text(session, key)?) {
                return Err(());
            }
        }
    }
    Ok(())
}

fn date(value: &str) -> Result<chrono::NaiveDate, ()> {
    let parsed = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| ())?;
    if parsed.to_string() != value || !value.starts_with("20") {
        return Err(());
    }
    Ok(parsed)
}

fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, ()> {
    // Validate, never normalize fractional legacy timestamp bytes.
    if value.len() > 35 {
        return Err(());
    }
    chrono::DateTime::parse_from_rfc3339(value).map_err(|_| ())
}

fn validate_transition(transition: &str, value: &Value) -> Result<(), ()> {
    let bytes = super::super::source_evidence::canonical(value).map_err(|_| ())?;
    if bytes.len()
        > if transition == "ENABLE" {
            MAX_PAGE_BYTES
        } else {
            MAX_WIRE_BYTES
        }
    {
        return Err(());
    }
    match transition {
        "ENABLE" => {
            exact_keys(
                value,
                &[
                    "current_cohort",
                    "process_binding",
                    "process_binding_sha256",
                    "evaluated_at",
                    "expires_at",
                    "session_evidence",
                    "expected_epoch",
                    "new_epoch",
                ],
            )?;
            validate_cohort(&value["current_cohort"])?;
            validate_process(&value["process_binding"])?;
            validate_sql_sessions(&value["session_evidence"])?;
            let epoch = i64_value(value, "expected_epoch")?;
            if Some(i64_value(value, "new_epoch")?) != epoch.checked_add(1)
                || value["process_binding"]["control_epoch"] != value["new_epoch"]
                || !digest(text(value, "process_binding_sha256")?)
                || super::super::source_evidence::digest(
                    &super::super::source_evidence::canonical(&value["process_binding"])
                        .map_err(|_| ())?,
                ) != text(value, "process_binding_sha256")?
                || timestamp(text(value, "evaluated_at")?)?
                    >= timestamp(text(value, "expires_at")?)?
            {
                return Err(());
            }
        }
        "DRAIN" => {
            exact_keys(value, &["schema", "expected_epoch", "reason_code"])?;
            if text(value, "schema")? != "owner-beta-paper-drain-v1" {
                return Err(());
            }
        }
        "OFF" | "RESTORED_OFF" => {
            exact_keys(
                value,
                if transition == "OFF" {
                    DRAIN_KEYS
                } else {
                    RESTORE_KEYS
                },
            )?;
            if text(value, "schema")?
                != if transition == "OFF" {
                    "owner-beta-paper-drain-receipt-v1"
                } else {
                    "owner-beta-paper-restore-receipt-v1"
                }
                || !["PAPER_STOP", "COMBINED_BACKUP"].contains(&text(value, "scope")?)
                || i64_value(value, "expected_epoch")?.checked_add(1)
                    != Some(i64_value(value, "resulting_epoch")?)
            {
                return Err(());
            }
            super::super::evidence::ClockTriple::from_value(&value["observed"]).map_err(|_| ())?;
        }
        "RECOVERY" => {
            let schema = text(value, "schema")?;
            if schema == "owner-beta-paper-recovery-v2" {
                exact_keys(value, RECOVERY_KEYS)?;
                if !["SETTLE_THEN_VALUE", "VALUE_ONLY"].contains(&text(value, "step")?)
                    || text(value, "reason_code")? != "EXPLICIT_RETAINED_RECOVERY"
                    || i64_value(value, "lineage_epoch")? >= i64_value(value, "active_epoch")?
                {
                    return Err(());
                }
            } else {
                exact_keys(
                    value,
                    &[
                        "schema",
                        "pending_target_id",
                        "effective_session",
                        "reason_code",
                        "decision_sha256",
                    ],
                )?;
                if schema != "owner-beta-paper-recovery-v1" {
                    return Err(());
                }
            }
        }
        _ => return Err(()),
    }
    for (key, child) in value.as_object().ok_or(())? {
        if key.ends_with("_id") || ["generation", "source_generation"].contains(&key.as_str()) {
            parse_uuid(child.as_str().ok_or(())?)?;
        } else if key.ends_with("_sha256") {
            if !digest(child.as_str().ok_or(())?) {
                return Err(());
            }
        } else if key.ends_with("_epoch")
            || ["source_epoch", "destination_epoch_floor"].contains(&key.as_str())
        {
            i64_value(value, key)?;
        } else if key == "effective_session" {
            date(child.as_str().ok_or(())?)?;
        } else if key == "evaluated_at" || key == "expires_at" {
            timestamp(child.as_str().ok_or(())?)?;
        } else if key == "request_key" {
            request_key(child.as_str().ok_or(())?)?;
        } else if key == "reason_code" {
            let reason = child.as_str().ok_or(())?;
            if reason.is_empty()
                || reason.len() > 64
                || !reason.as_bytes()[0].is_ascii_uppercase()
                || !reason
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(());
            }
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResetKind {
    Configured,
    Off,
    RestoredOff,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ResetBoundary {
    pub account_id: Uuid,
    pub event_id: Uuid,
    pub kind: ResetKind,
    pub control_epoch: i64,
    pub occurred_at_us: i64,
    pub payload_sha256: String,
}

impl ResetBoundary {
    pub fn parse(value: &Value) -> Result<Self, ()> {
        exact_keys(
            value,
            &[
                "account_id",
                "event_id",
                "kind",
                "control_epoch",
                "occurred_at_us",
                "payload_sha256",
            ],
        )?;
        let kind = match text(value, "kind")? {
            "CONFIGURED" => ResetKind::Configured,
            "OFF" => ResetKind::Off,
            "RESTORED_OFF" => ResetKind::RestoredOff,
            _ => return Err(()),
        };
        let control_epoch = i64_value(value, "control_epoch")?;
        if (kind == ResetKind::Configured) != (control_epoch == 0)
            || !digest(text(value, "payload_sha256")?)
        {
            return Err(());
        }
        Ok(Self {
            account_id: parse_uuid(text(value, "account_id")?)?,
            event_id: parse_uuid(text(value, "event_id")?)?,
            kind,
            control_epoch,
            occurred_at_us: i64_value(value, "occurred_at_us")?,
            payload_sha256: text(value, "payload_sha256")?.to_owned(),
        })
    }
}

pub(super) struct ResetSelection {
    pub installation_id: Uuid,
    pub reset: ResetBoundary,
    pub release_manifest_sha256: String,
    pub cohort_sha256: String,
}

impl ResetSelection {
    pub fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let value = parse_canonical(bytes)?;
        exact_keys(
            &value,
            &[
                "schema",
                "installation_id",
                "reset",
                "release_manifest_sha256",
                "cohort_sha256",
            ],
        )?;
        if text(&value, "schema")? != "owner-beta-paper-reset-selection-v1" {
            return Err(());
        }
        Self::fields(&value)
    }

    fn fields(value: &Value) -> Result<Self, ()> {
        if !digest(text(value, "release_manifest_sha256")?)
            || !digest(text(value, "cohort_sha256")?)
        {
            return Err(());
        }
        Ok(Self {
            installation_id: parse_uuid(text(value, "installation_id")?)?,
            reset: ResetBoundary::parse(&value["reset"])?,
            release_manifest_sha256: text(value, "release_manifest_sha256")?.to_owned(),
            cohort_sha256: text(value, "cohort_sha256")?.to_owned(),
        })
    }
}

pub(super) struct ResetAnchor {
    pub selection: ResetSelection,
    pub boot_id: Uuid,
    pub observed: super::super::evidence::ClockTriple,
}

impl ResetAnchor {
    pub fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let value = parse_canonical(bytes)?;
        exact_keys(
            &value,
            &[
                "schema",
                "installation_id",
                "reset",
                "release_manifest_sha256",
                "cohort_sha256",
                "boot_id",
                "observed",
            ],
        )?;
        if text(&value, "schema")? != "owner-beta-paper-reset-anchor-v1" {
            return Err(());
        }
        let selection = ResetSelection::fields(&value)?;
        let observed =
            super::super::evidence::ClockTriple::from_value(&value["observed"]).map_err(|_| ())?;
        if selection
            .reset
            .occurred_at_us
            .checked_mul(1000)
            .is_none_or(|ns| observed.realtime_ns() <= ns)
        {
            return Err(());
        }
        Ok(Self {
            selection,
            boot_id: parse_uuid(text(&value, "boot_id")?)?,
            observed,
        })
    }
}

pub(super) struct ExactReadback {
    pub value: Value,
}

impl ExactReadback {
    pub fn correlate_original(&self, intent: &Intent) -> Result<(), ()> {
        if self.value["status"] == "COMMITTED" && self.value["kind"] == "RECONCILE" {
            let result = embedded(&self.value, "result_canonical", MAX_PAGE_BYTES)?;
            if result["data"]["summary"]["installation_id"] != intent.installation_id.to_string() {
                return Err(());
            }
        }
        Ok(())
    }

    pub fn parse(value: Value, request: &ExactRequest) -> Result<Self, ()> {
        let bytes = super::super::source_evidence::canonical(&value).map_err(|_| ())?;
        let value = parse_canonical_limit(&bytes, MAX_EXACT_BYTES)?;
        exact_keys(
            &value,
            &[
                "schema",
                "kind",
                "request_sha256",
                "status",
                "account_id",
                "current_epoch",
                "current_state",
                "event_id",
                "event_epoch",
                "result_canonical",
                "result_sha256",
                "reset",
            ],
        )?;
        if text(&value, "schema")? != "owner-beta-paper-exact-readback-v1"
            || value["kind"] != request.value["kind"]
            || text(&value, "request_sha256")? != request.sha256
        {
            return Err(());
        }
        let account = optional_uuid(&value, "account_id")?;
        let original_account = optional_uuid(&request.value, "account_id")?;
        if original_account.is_some() && account != original_account {
            return Err(());
        }
        if account.is_none() {
            if request.value["kind"] != "CONFIGURE"
                || text(&value, "status")? != "ABSENT"
                || !value["current_epoch"].is_null()
                || !value["current_state"].is_null()
            {
                return Err(());
            }
        } else {
            i64_value(&value, "current_epoch")?;
            if !["OFF", "ENABLED", "DRAINING", "RESTORED_OFF", "BLOCKED"]
                .contains(&text(&value, "current_state")?)
            {
                return Err(());
            }
        }
        match text(&value, "status")? {
            "COMMITTED" => {
                if request.value["kind"] == "RESET" || !value["reset"].is_null() {
                    return Err(());
                }
                parse_uuid(text(&value, "event_id")?)?;
                let epoch = i64_value(&value, "event_epoch")?;
                if epoch > i64_value(&value, "current_epoch")? {
                    return Err(());
                }
                let result = embedded(&value, "result_canonical", MAX_PAGE_BYTES)?;
                if !digest(text(&value, "result_sha256")?)
                    || super::super::source_evidence::digest(
                        text(&value, "result_canonical")?.as_bytes(),
                    ) != text(&value, "result_sha256")?
                {
                    return Err(());
                }
                validate_original_result(&result, request, &value)?;
            }
            "ABSENT" | "UNPROVEN" => {
                if request.value["kind"] == "RESET" {
                    return Err(());
                }
                for key in [
                    "event_id",
                    "event_epoch",
                    "result_canonical",
                    "result_sha256",
                    "reset",
                ] {
                    if !value[key].is_null() {
                        return Err(());
                    }
                }
            }
            "RESET" => {
                if request.value["kind"] != "RESET"
                    || account.is_none()
                    || value["current_epoch"] != request.value["args"]["expected_epoch"]
                {
                    return Err(());
                }
                for key in [
                    "event_id",
                    "event_epoch",
                    "result_canonical",
                    "result_sha256",
                ] {
                    if !value[key].is_null() {
                        return Err(());
                    }
                }
                let reset = ResetBoundary::parse(&value["reset"])?;
                if Some(reset.account_id) != account
                    || reset.control_epoch > i64_value(&value, "current_epoch")?
                {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
        Ok(Self { value })
    }
}

fn validate_original_result(
    result: &Value,
    request: &ExactRequest,
    readback: &Value,
) -> Result<(), ()> {
    exact_keys(
        result,
        &[
            "schema",
            "action",
            "outcome",
            "control_epoch",
            "account_id",
            "data",
        ],
    )?;
    if text(result, "schema")? != "owner-beta-paper-action-v1"
        || text(result, "outcome")? != "OK"
        || result["account_id"] != readback["account_id"]
        || result["control_epoch"] != readback["event_epoch"]
    {
        return Err(());
    }
    let args = &request.value["args"];
    let data = &result["data"];
    let action = text(&request.value, "kind")?;
    let expected_action = match action {
        "BEGIN" => "BEGIN_OPERATION",
        "RECONCILE" => "RECONCILE_OPERATION",
        _ => action,
    };
    if text(result, "action")? != expected_action {
        return Err(());
    }
    let expected_epoch = match action {
        "CONFIGURE" => {
            exact_keys(
                data,
                &[
                    "account_id",
                    "event_id",
                    "state",
                    "initial_cash",
                    "currency",
                ],
            )?;
            if data["account_id"] != result["account_id"]
                || data["state"] != "OFF"
                || data["initial_cash"] != "10000000.0000"
                || data["currency"] != "KRW"
            {
                return Err(());
            }
            0
        }
        "BEGIN" => {
            exact_keys(
                data,
                &[
                    "generation",
                    "operation_event_id",
                    "intent_sha256",
                    "fenced_epoch",
                ],
            )?;
            let intent = embedded(args, "intent_canonical", MAX_WIRE_BYTES)?;
            let epoch = i64_value(args, "expected_epoch")?
                .max(i64_value(&intent, "local_epoch_floor")?)
                .checked_add(1)
                .ok_or(())?;
            if data["generation"] != intent["generation"]
                || data["fenced_epoch"].as_i64() != Some(epoch)
                || data["operation_event_id"] != readback["event_id"]
                || text(data, "intent_sha256")?
                    != super::super::source_evidence::digest(
                        text(args, "intent_canonical")?.as_bytes(),
                    )
            {
                return Err(());
            }
            epoch
        }
        "RECONCILE" => {
            exact_keys(data, &["observation_id", "summary", "summary_sha256"])?;
            validate_summary(&data["summary"])?;
            let summary = &data["summary"];
            if data["observation_id"] != readback["event_id"]
                || summary["observation_id"] != readback["event_id"]
                || summary["account_id"] != result["account_id"]
                || summary["generation"] != args["generation"]
                || summary["inventory_sha256"] != args["inventory_sha256"]
                || summary["control_epoch"] != args["expected_epoch"]
                || text(data, "summary_sha256")?
                    != super::super::source_evidence::digest(
                        &super::super::source_evidence::canonical(summary).map_err(|_| ())?,
                    )
            {
                return Err(());
            }
            i64_value(args, "expected_epoch")?
        }
        "TRANSITION" => {
            let transition = text(args, "transition")?;
            let evidence = embedded(args, "evidence_canonical", MAX_PAGE_BYTES)?;
            if transition == "RECOVERY" && evidence["schema"] == "owner-beta-paper-recovery-v2" {
                exact_keys(
                    data,
                    &["event_id", "state", "pending_target_id", "grant_id"],
                )?;
                parse_uuid(text(data, "grant_id")?)?;
                if data["pending_target_id"] != evidence["pending_target_id"] {
                    return Err(());
                }
            } else {
                exact_keys(data, &["event_id", "state"])?;
            }
            let state = match transition {
                "ENABLE" => "ENABLED",
                "DRAIN" => "DRAINING",
                "OFF" => "OFF",
                "RESTORED_OFF" => "RESTORED_OFF",
                "RECOVERY" => text(data, "state")?,
                _ => return Err(()),
            };
            if data["state"] != state
                || !["OFF", "ENABLED", "DRAINING", "RESTORED_OFF"].contains(&state)
            {
                return Err(());
            }
            i64_value(args, "expected_epoch")?
                .checked_add(if transition == "RECOVERY" { 0 } else { 1 })
                .ok_or(())?
        }
        _ => return Err(()),
    };
    if i64_value(result, "control_epoch")? != expected_epoch {
        return Err(());
    }
    if ["CONFIGURE", "TRANSITION"].contains(&action) && data["event_id"] != readback["event_id"] {
        return Err(());
    }
    Ok(())
}

fn validate_summary(value: &Value) -> Result<(), ()> {
    exact_keys(
        value,
        &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "observation_id",
            "control_epoch",
            "inventory_sha256",
            "observed_at",
            "database_sha256",
            "ledger_sha256",
            "pending_sha256",
            "outbox_sha256",
            "jobs_sha256",
            "unresolved_commits",
            "live_claims",
            "held_targets",
            "quarantined_deliveries",
        ],
    )?;
    if text(value, "schema")? != "owner-beta-paper-reconciliation-v1" {
        return Err(());
    }
    for key in [
        "installation_id",
        "generation",
        "account_id",
        "observation_id",
    ] {
        parse_uuid(text(value, key)?)?;
    }
    for key in [
        "inventory_sha256",
        "database_sha256",
        "ledger_sha256",
        "pending_sha256",
        "outbox_sha256",
        "jobs_sha256",
    ] {
        if !digest(text(value, key)?) {
            return Err(());
        }
    }
    for key in [
        "control_epoch",
        "unresolved_commits",
        "live_claims",
        "held_targets",
        "quarantined_deliveries",
    ] {
        i64_value(value, key)?;
    }
    chrono::NaiveDateTime::parse_from_str(text(value, "observed_at")?, "%Y-%m-%dT%H:%M:%SZ")
        .map_err(|_| ())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CLOCK_KEYS, CONFIGURE_KEYS, DRAIN_KEYS, INTENT_KEYS, INVENTORY_KEYS, RECOVERY_KEYS,
        RESTORE_KEYS, WRITER_KEYS, parse_artifact, parse_canonical, parse_capability,
        parse_configure, parse_intent, parse_process_binding,
    };
    use super::{ExactReadback, ExactRequest, ResetAnchor, ResetBoundary, ResetSelection};
    use crate::owner_beta::paper::operations::Verb;
    use serde_json::{Value, json};

    fn canonical(v: serde_json::Value) -> Vec<u8> {
        crate::owner_beta::paper::source_evidence::canonical(&v).unwrap()
    }

    // Untrusted parser examples only. None constructs a CheckedEnableSession,
    // CheckedArmOrigin, retained Source reader, or checked anchor.
    fn enable_example() -> (ExactRequest, super::Intent, super::Artifact) {
        let h = |c: char| format!("sha256:{}", c.to_string().repeat(64));
        let cohort = json!({
            "owner_user_id":"11111111-1111-4111-8111-111111111111",
            "strategy_config_id":"55555555-5555-4555-8555-555555555555",
            "policy_sha256":h('a'), "activation_epoch":1, "config_epoch":1,
            "strategy_config_sha256":h('b'), "entitlement_reference":"fixture-only",
            "entitlement_sha256":h('c'), "cohort_sha256":h('d'),
            "release_manifest_sha256":h('a'), "research_image_id":h('e'),
            "helper_image_id":h('f'), "semantic_config_sha256":h('1'),
            "calendar_sha256":h('2'), "verifier_revision":"b".repeat(40)
        });
        let process = json!({
            "schema":"owner-beta-paper-process-v1", "owner_user_id":cohort["owner_user_id"],
            "account_id":"44444444-4444-4444-8444-444444444444",
            "strategy_config_id":cohort["strategy_config_id"],
            "consent_event_id":"66666666-6666-4666-8666-666666666666",
            "consent_version":"owner-beta-paper-v1-draft-1", "control_epoch":1,
            "daily_cohort_sha256":cohort["cohort_sha256"],
            "financial_contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
            "code_commit":"b".repeat(40), "release_manifest_sha256":h('a'),
            "api_binary_sha256":h('3'), "worker_binary_sha256":h('4')
        });
        let bytes = canonical(process.clone());
        let hash = crate::owner_beta::paper::source_evidence::digest(&bytes);
        let original = parse_intent(
            &intent(
                Verb::Enable,
                "ENABLE",
                Some(&hash),
                Some("44444444-4444-4444-8444-444444444444"),
            ),
            Verb::Enable,
        )
        .unwrap();
        let artifact = parse_process_binding(&bytes, &original).unwrap();
        let sessions = ["2026-11-02", "2026-11-03", "2026-11-04"]
            .iter()
            .enumerate()
            .map(|(i, date)| {
                let run_id = format!("77777777-7777-4777-8777-{:012}", i + 1);
                json!({"session_date":date, "run_id":run_id, "admission_sha256":h('5'),
                "source_event_sha256":h('6'), "root_event_sha256":h('7')})
            })
            .collect::<Vec<_>>();
        let evidence = json!({
            "current_cohort":cohort, "process_binding":process, "process_binding_sha256":hash,
            "evaluated_at":"2026-11-06T01:00:00.123456789Z",
            "expires_at":"2026-11-06T01:02:00.123456789Z",
            "session_evidence":sessions, "expected_epoch":0, "new_epoch":1
        });
        let request = ExactRequest::from_value(json!({
            "schema":"owner-beta-paper-exact-request-v1", "kind":"TRANSITION",
            "account_id":original.account_id, "args":{"expected_epoch":0, "transition":"ENABLE",
                "evidence_canonical":String::from_utf8(canonical(evidence)).unwrap(),
                "request_key":original.request_key}
        }))
        .unwrap();
        (request, original, artifact)
    }

    fn enable_readback(request: &ExactRequest) -> Value {
        let result = json!({"schema":"owner-beta-paper-action-v1", "action":"TRANSITION", "outcome":"OK",
            "control_epoch":1, "account_id":request.value["account_id"],
            "data":{"event_id":"88888888-8888-4888-8888-888888888888","state":"ENABLED"}});
        let bytes = canonical(result);
        json!({"schema":"owner-beta-paper-exact-readback-v1", "kind":"TRANSITION",
            "request_sha256":request.sha256, "status":"COMMITTED",
            "account_id":request.value["account_id"], "current_epoch":9, "current_state":"OFF",
            "event_id":"88888888-8888-4888-8888-888888888888", "event_epoch":1,
            "result_canonical":String::from_utf8(bytes.clone()).unwrap(),
            "result_sha256":crate::owner_beta::paper::source_evidence::digest(&bytes), "reset":null})
    }

    #[test]
    fn exact_request_recursively_rejects_duplicates_depth_sizes_and_unknown_keys() {
        let (request, _, _) = enable_example();
        assert!(ExactRequest::parse(request.canonical.as_bytes()).is_ok());
        let mut changed = request.value.clone();
        changed["args"]["evidence_canonical"] = json!("{\"a\":true,\"a\":false}");
        assert!(ExactRequest::from_value(changed).is_err());
        let duplicate_outer =
            request
                .canonical
                .replacen("\"kind\":", "\"kind\":\"TRANSITION\",\"kind\":", 1);
        assert!(ExactRequest::parse(duplicate_outer.as_bytes()).is_err());
        let mut changed = request.value.clone();
        changed["args"]["evidence_canonical"] =
            json!(format!("{}0{}", "[".repeat(16), "]".repeat(16)));
        assert!(ExactRequest::from_value(changed).is_err());
        let mut changed = request.value.clone();
        changed["args"]["evidence_canonical"] = json!(" ".repeat(super::MAX_PAGE_BYTES + 1));
        assert!(ExactRequest::from_value(changed).is_err());
        assert!(ExactRequest::parse(&vec![b' '; super::MAX_EXACT_BYTES + 1]).is_err());
        for extra in ["actor", "credential", "sql", "path"] {
            let mut changed = request.value.clone();
            changed["args"][extra] = json!("forbidden");
            assert!(ExactRequest::from_value(changed).is_err());
        }
    }

    #[test]
    fn exact_enable_preserves_original_fractional_times_without_current_or_expiry_check() {
        let (request, original, artifact) = enable_example();
        assert!(request.correlate(&original, Some(&artifact)).is_ok());
        assert!(request.canonical.contains("01:00:00.123456789Z"));
        let mut same_instant_changed_bytes = request.value.clone();
        let text = same_instant_changed_bytes["args"]["evidence_canonical"]
            .as_str()
            .unwrap()
            .replace(".123456789Z", ".123456789+00:00");
        same_instant_changed_bytes["args"]["evidence_canonical"] = json!(text);
        let different = ExactRequest::from_value(same_instant_changed_bytes).unwrap();
        assert_ne!(different.canonical, request.canonical);
        assert_ne!(different.sha256, request.sha256);
        let readback = ExactReadback::parse(enable_readback(&request), &request).unwrap();
        assert_eq!(readback.value["current_epoch"], 9);
        assert_eq!(readback.value["current_state"], "OFF");
        let result: Value =
            serde_json::from_str(readback.value["result_canonical"].as_str().unwrap()).unwrap();
        assert_eq!(result["control_epoch"], 1);
        assert_eq!(result["data"]["state"], "ENABLED");
    }

    #[test]
    fn exact_readback_rejects_full_identity_hash_and_historical_result_swaps() {
        let (request, original, artifact) = enable_example();
        for (key, replacement) in [
            ("kind", json!("BEGIN")),
            (
                "request_sha256",
                json!(format!("sha256:{}", "0".repeat(64))),
            ),
            ("account_id", json!("99999999-9999-4999-8999-999999999999")),
            ("event_id", json!("99999999-9999-4999-8999-999999999999")),
            ("event_epoch", json!(2)),
            ("result_sha256", json!(format!("sha256:{}", "0".repeat(64)))),
            ("current_epoch", json!(0)),
            ("extra", json!(false)),
        ] {
            let mut changed = enable_readback(&request);
            changed[key] = replacement;
            assert!(ExactReadback::parse(changed, &request).is_err(), "{key}");
        }
        for key in ["account_id", "control_epoch"] {
            let mut changed = enable_readback(&request);
            let mut result: Value =
                serde_json::from_str(changed["result_canonical"].as_str().unwrap()).unwrap();
            result[key] = if key == "account_id" {
                json!("99999999-9999-4999-8999-999999999999")
            } else {
                json!(2)
            };
            let bytes = canonical(result);
            changed["result_canonical"] = json!(String::from_utf8(bytes.clone()).unwrap());
            changed["result_sha256"] =
                json!(crate::owner_beta::paper::source_evidence::digest(&bytes));
            assert!(ExactReadback::parse(changed, &request).is_err(), "{key}");
        }
        for (key, value) in [
            ("expected_epoch", json!(1)),
            ("request_key", json!("different")),
        ] {
            let mut changed = request.value.clone();
            changed["args"][key] = value;
            let changed = ExactRequest::from_value(changed).unwrap();
            assert!(changed.correlate(&original, Some(&artifact)).is_err());
        }
    }

    #[test]
    fn absent_and_legacy_unproven_cannot_be_reset_or_committed() {
        let (request, _, _) = enable_example();
        for status in ["ABSENT", "UNPROVEN"] {
            let mut value = enable_readback(&request);
            value["status"] = json!(status);
            for key in [
                "event_id",
                "event_epoch",
                "result_canonical",
                "result_sha256",
            ] {
                value[key] = Value::Null;
            }
            assert!(ExactReadback::parse(value.clone(), &request).is_ok());
            value["event_id"] = json!("88888888-8888-4888-8888-888888888888");
            assert!(ExactReadback::parse(value, &request).is_err());
        }
    }

    #[test]
    fn changed_session_preimages_cannot_read_back_the_original_receipt() {
        let (request, _, _) = enable_example();
        for (key, replacement) in [
            ("run_id", json!("99999999-9999-4999-8999-999999999999")),
            ("session_date", json!("2026-10-30")),
            (
                "admission_sha256",
                json!(format!("sha256:{}", "9".repeat(64))),
            ),
            (
                "source_event_sha256",
                json!(format!("sha256:{}", "9".repeat(64))),
            ),
            (
                "root_event_sha256",
                json!(format!("sha256:{}", "9".repeat(64))),
            ),
        ] {
            let mut changed = request.value.clone();
            let mut evidence: Value =
                serde_json::from_str(changed["args"]["evidence_canonical"].as_str().unwrap())
                    .unwrap();
            evidence["session_evidence"][0][key] = replacement;
            changed["args"]["evidence_canonical"] =
                json!(String::from_utf8(canonical(evidence)).unwrap());
            let changed = ExactRequest::from_value(changed).unwrap();
            assert!(
                ExactReadback::parse(enable_readback(&request), &changed).is_err(),
                "{key}"
            );
        }
        for forbidden in [
            "slot_id",
            "job_id",
            "attempt_id",
            "reservation_id",
            "arm",
            "cohort",
        ] {
            let mut changed = request.value.clone();
            let mut evidence: Value =
                serde_json::from_str(changed["args"]["evidence_canonical"].as_str().unwrap())
                    .unwrap();
            evidence["session_evidence"][0][forbidden] = json!("untrusted");
            changed["args"]["evidence_canonical"] =
                json!(String::from_utf8(canonical(evidence)).unwrap());
            assert!(ExactRequest::from_value(changed).is_err(), "{forbidden}");
        }
    }

    #[test]
    fn exact_request_accepts_all_five_families_and_each_closed_transition_variant() {
        let (enable, _, _) = enable_example();
        let account = enable.value["account_id"].clone();
        let generation = "22222222-2222-4222-8222-222222222222";
        let hash = format!("sha256:{}", "a".repeat(64));
        let contract = json!({"contract_id":"owner-beta-paper-v1-draft-1",
            "contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
            "decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
            "cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1});
        let cases = [
            json!({"schema":"owner-beta-paper-exact-request-v1","kind":"CONFIGURE","account_id":null,
                "args":{"strategy_config_id":account,"account_name":"Owner-beta Paper",
                    "contract_canonical":String::from_utf8(canonical(contract)).unwrap(),"process_binding":null,"request_key":"original"}}),
            json!({"schema":"owner-beta-paper-exact-request-v1","kind":"BEGIN","account_id":account,
                "args":{"expected_epoch":0,"intent_canonical":String::from_utf8(intent(Verb::Begin,"STOP",None,account.as_str())).unwrap(),
                    "request_key":format!("{generation}:begin")}}),
            json!({"schema":"owner-beta-paper-exact-request-v1","kind":"RECONCILE","account_id":account,
                "args":{"generation":generation,"expected_epoch":1,"inventory_sha256":hash,"request_key":format!("{generation}:reconcile:1")}}),
            json!({"schema":"owner-beta-paper-exact-request-v1","kind":"RESET","account_id":account,"args":{"expected_epoch":1}}),
        ];
        for case in cases {
            let parsed = ExactRequest::from_value(case).unwrap();
            assert!(ExactRequest::parse(parsed.canonical.as_bytes()).is_ok());
        }
        let off = json!({"schema":"owner-beta-paper-drain-receipt-v1",
            "installation_id":"11111111-1111-4111-8111-111111111111",
            "generation":generation,"account_id":account,"request_key":format!("{generation}:off"),
            "scope":"PAPER_STOP","expected_epoch":1,"resulting_epoch":2,
            "intent_sha256":hash,"stage_sha256":hash,"inventory_sha256":hash,"reconciliation_sha256":hash,
            "observation_id":"33333333-3333-4333-8333-333333333333","release_manifest_sha256":hash,
            "boot_id":"55555555-5555-4555-8555-555555555555","observed":{"realtime_ns":1,"monotonic_ns":1,"boottime_ns":1}});
        let mut restore = off.clone();
        restore["schema"] = json!("owner-beta-paper-restore-receipt-v1");
        for (key, val) in [
            (
                "source_installation_id",
                json!("66666666-6666-4666-8666-666666666666"),
            ),
            (
                "source_generation",
                json!("77777777-7777-4777-8777-777777777777"),
            ),
            ("source_epoch", json!(1)),
            ("archive_sha256", json!(hash)),
            ("source_audit_sha256", json!(hash)),
            ("restored_audit_sha256", json!(hash)),
            ("destination_epoch_floor", json!(1)),
        ] {
            restore[key] = val;
        }
        let legacy_recovery = json!({"schema":"owner-beta-paper-recovery-v1",
            "pending_target_id":"88888888-8888-4888-8888-888888888888",
            "effective_session":"2026-11-04","reason_code":"EXPLICIT_RECOVERY","decision_sha256":hash});
        let recovery = json!({"schema":"owner-beta-paper-recovery-v2",
            "installation_id":"11111111-1111-4111-8111-111111111111","generation":generation,"account_id":account,
            "request_key":format!("{generation}:recover"),"pending_target_id":"88888888-8888-4888-8888-888888888888",
            "lineage_id":"99999999-9999-4999-8999-999999999999","effective_session":"2026-11-04",
            "lineage_epoch":0,"active_epoch":1,"original_receipt_sha256":hash,
            "consent_event_id":"33333333-3333-4333-8333-333333333333","process_config_sha256":hash,
            "step":"VALUE_ONLY","signal_admission_sha256":hash,"execution_admission_sha256":hash,
            "decision_sha256":hash,"reason_code":"EXPLICIT_RETAINED_RECOVERY","expires_at":"2026-11-07T00:00:00Z"});
        for (transition, evidence) in [
            (
                "DRAIN",
                json!({"schema":"owner-beta-paper-drain-v1","expected_epoch":1,"reason_code":"EXPLICIT_STOP"}),
            ),
            ("OFF", off),
            ("RESTORED_OFF", restore),
            ("RECOVERY", legacy_recovery),
            ("RECOVERY", recovery),
        ] {
            assert!(ExactRequest::from_value(json!({"schema":"owner-beta-paper-exact-request-v1",
                "kind":"TRANSITION","account_id":account,"args":{"expected_epoch":1,"transition":transition,
                    "evidence_canonical":String::from_utf8(canonical(evidence)).unwrap(),"request_key":"original"}})).is_ok(),"{transition}");
        }
    }

    #[test]
    fn reset_selection_and_anchor_are_closed_exact_microseconds_and_all_three_clocks() {
        let h = format!("sha256:{}", "a".repeat(64));
        let reset = json!({"account_id":"11111111-1111-4111-8111-111111111111",
            "event_id":"22222222-2222-4222-8222-222222222222","kind":"OFF",
            "control_epoch":7,"occurred_at_us":1234567,"payload_sha256":h});
        assert_eq!(
            ResetBoundary::parse(&reset).unwrap().occurred_at_us,
            1234567
        );
        let selection = json!({"schema":"owner-beta-paper-reset-selection-v1",
            "installation_id":"33333333-3333-4333-8333-333333333333","reset":reset,
            "release_manifest_sha256":h,"cohort_sha256":h});
        assert!(ResetSelection::parse(&canonical(selection.clone())).is_ok());
        let mut anchor = selection;
        anchor["schema"] = json!("owner-beta-paper-reset-anchor-v1");
        anchor["boot_id"] = json!("44444444-4444-4444-8444-444444444444");
        anchor["observed"] = json!({"realtime_ns":1234567001_i64,"monotonic_ns":2,"boottime_ns":2});
        assert!(ResetAnchor::parse(&canonical(anchor.clone())).is_ok());
        for replacement in [
            json!(1234567000_i64),
            json!(1234566999_i64),
            json!(-1),
            json!(true),
        ] {
            let mut bad = anchor.clone();
            bad["observed"]["realtime_ns"] = replacement;
            assert!(ResetAnchor::parse(&canonical(bad)).is_err());
        }
        for key in ["monotonic_ns", "boottime_ns"] {
            let mut bad = anchor.clone();
            bad["observed"][key] = json!(-1);
            assert!(ResetAnchor::parse(&canonical(bad)).is_err());
        }
        let mut bad = anchor.clone();
        bad["reset"]["occurred_at_us"] = json!(i64::MAX);
        assert!(ResetAnchor::parse(&canonical(bad)).is_err());
        let mut bad = anchor.clone();
        bad["observed"]["foreign_clock"] = json!(0);
        assert!(ResetAnchor::parse(&canonical(bad)).is_err());
        let mut bad = anchor;
        bad["boot_id"] = json!("00000000-0000-0000-0000-000000000000");
        assert!(ResetAnchor::parse(&canonical(bad)).is_err());
    }

    fn intent(
        verb: Verb,
        purpose: &str,
        artifact_sha: Option<&str>,
        account: Option<&str>,
    ) -> Vec<u8> {
        let request_key = match verb {
            Verb::Begin => "22222222-2222-4222-8222-222222222222:begin",
            Verb::Reconcile => "22222222-2222-4222-8222-222222222222:reconcile:1",
            Verb::Off => "22222222-2222-4222-8222-222222222222:off",
            Verb::RestoreOff => "22222222-2222-4222-8222-222222222222:restore-off",
            Verb::Recover => "22222222-2222-4222-8222-222222222222:recover",
            Verb::Enable => "22222222-2222-4222-8222-222222222222:enable",
            Verb::Configure | Verb::Inspect | Verb::Readback | Verb::ObserveReset => "request",
        };
        canonical(
            json!({"schema":"owner-beta-paper-operation-intent-v1", "installation_id":"11111111-1111-4111-8111-111111111111", "generation":"22222222-2222-4222-8222-222222222222", "account_id":account, "predecessor_generation":null, "request_key":request_key, "purpose":purpose, "scope":"PAPER_STOP", "expected_epoch":0, "local_epoch_floor":0, "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)), "artifact_sha256":artifact_sha, "resume_target_id":null}),
        )
    }

    #[test]
    fn rejects_noncanonical_types_overflow_unknown_keys_and_depth() {
        assert!(parse_canonical(br#"{"a":true,"a":false}"#).is_err());
        assert!(parse_canonical(br#"{"a":1 }"#).is_err());
        assert!(parse_canonical(br#"{"a":9223372036854775808}"#).is_err());
        assert!(parse_canonical(br#"{"a":1,"x":null}"#).is_ok());
        let deep = format!("{}0{}", "[".repeat(16), "]".repeat(16));
        assert!(parse_canonical(deep.as_bytes()).is_err());
        assert!(parse_canonical(br#"{"a":"x\n"}"#).is_err());
    }

    #[test]
    fn intent_is_closed_and_verb_bound() {
        let bytes = intent(
            Verb::Begin,
            "STOP",
            None,
            Some("33333333-3333-4333-8333-333333333333"),
        );
        let parsed = parse_intent(&bytes, Verb::Begin).unwrap();
        assert_eq!(parsed.purpose, "STOP");
        assert!(parse_intent(&bytes, Verb::Off).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["unexpected"] = json!(false);
        assert!(parse_intent(&canonical(value), Verb::Begin).is_err());
    }

    #[test]
    fn configure_contract_is_exact_and_cannot_forge_consent() {
        let good = json!({
            "schema":"owner-beta-paper-configure-operation-v1",
            "strategy_config_id":"33333333-3333-4333-8333-333333333333",
            "account_name":"Owner-beta Paper",
            "contract":{"contract_id":"owner-beta-paper-v1-draft-1","contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0","decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394","cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1}
        });
        assert!(parse_configure(&canonical(good.clone())).is_ok());
        let mut forged = good;
        forged["consent"] = json!(true);
        assert!(parse_configure(&canonical(forged)).is_err());
    }

    #[test]
    fn process_binding_is_closed_and_next_epoch_bound() {
        let process = json!({
            "schema":"owner-beta-paper-process-v1",
            "owner_user_id":"33333333-3333-4333-8333-333333333333",
            "account_id":"44444444-4444-4444-8444-444444444444",
            "strategy_config_id":"55555555-5555-4555-8555-555555555555",
            "consent_event_id":"66666666-6666-4666-8666-666666666666",
            "consent_version":"owner-beta-paper-v1-draft-1",
            "control_epoch":1,
            "daily_cohort_sha256":format!("sha256:{}", "b".repeat(64)),
            "financial_contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
            "code_commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "api_binary_sha256":format!("sha256:{}", "d".repeat(64)),
            "worker_binary_sha256":format!("sha256:{}", "e".repeat(64))
        });
        let bytes = canonical(process.clone());
        let artifact_hash = crate::owner_beta::paper::source_evidence::digest(&bytes);
        let request = intent(
            Verb::Enable,
            "ENABLE",
            Some(&artifact_hash),
            Some("44444444-4444-4444-8444-444444444444"),
        );
        let intent = parse_intent(&request, Verb::Enable).unwrap();
        assert!(parse_process_binding(&bytes, &intent).is_ok());
        let mut wrong_epoch = process.clone();
        wrong_epoch["control_epoch"] = json!(0);
        assert!(parse_process_binding(&canonical(wrong_epoch), &intent).is_err());
        let mut widened = process;
        widened["operator_permit"] = json!(true);
        assert!(parse_process_binding(&canonical(widened), &intent).is_err());
    }

    #[test]
    fn capability_is_closed_and_pins_native_and_serving_elfs() {
        let capability = json!({
            "schema":"owner-beta-paper-reader-capability-v1",
            "minimum":1,
            "code_commit":"b".repeat(40),
            "release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),
            "operations_schema_sha256":format!("sha256:{}", "c".repeat(64)),
            "archive_schema_sha256":format!("sha256:{}", "d".repeat(64)),
            "migration_0068_sha256":format!("sha256:{}", "e".repeat(64)),
            "audit_sql_sha256":format!("sha256:{}", "f".repeat(64)),
            "api_binary_sha256":format!("sha256:{}", "1".repeat(64)),
            "worker_binary_sha256":format!("sha256:{}", "2".repeat(64)),
            "native_binary_sha256":format!("sha256:{}", "3".repeat(64))
        });
        assert!(parse_capability(&canonical(capability.clone())).is_ok());
        let mut widened = capability.clone();
        widened["verified"] = json!(true);
        assert!(parse_capability(&canonical(widened)).is_err());
        let mut wrong_floor = capability;
        wrong_floor["minimum"] = json!(0);
        assert!(parse_capability(&canonical(wrong_floor)).is_err());
    }

    #[test]
    fn inventory_requires_all_writer_and_identity_pins() {
        let artifact_hash = format!("sha256:{}", "b".repeat(64));
        let intent_bytes = intent(
            Verb::Reconcile,
            "STOP",
            Some(&artifact_hash),
            Some("33333333-3333-4333-8333-333333333333"),
        );
        let mut parsed = parse_intent(&intent_bytes, Verb::Reconcile).unwrap();
        let inventory = json!({
            "schema":"owner-beta-paper-writer-inventory-v1","installation_id":"11111111-1111-4111-8111-111111111111","generation":"22222222-2222-4222-8222-222222222222","account_id":"33333333-3333-4333-8333-333333333333","scope":"PAPER_STOP","fence_event_id":"44444444-4444-4444-8444-444444444444","control_epoch":1,"boot_id":"55555555-5555-4555-8555-555555555555","observed":{"realtime_ns":1,"monotonic_ns":1,"boottime_ns":1},"release_manifest_sha256":format!("sha256:{}", "a".repeat(64)),"writers":[],"launch_fence_sha256":format!("sha256:{}", "c".repeat(64)),"backend_observation_sha256":format!("sha256:{}", "d".repeat(64))
        });
        let bytes = canonical(inventory);
        parsed.artifact_sha256 = Some(crate::owner_beta::paper::source_evidence::digest(&bytes));
        assert!(parse_artifact(&bytes, "owner-beta-paper-writer-inventory-v1", &parsed).is_ok());
    }

    #[test]
    fn typed_closed_key_sets_match_both_adopted_schemas() {
        let operations: Value = serde_json::from_str(include_str!(
            "../../../../../../docs/contracts/owner-beta-paper-operations-v1.schema.json"
        ))
        .unwrap();
        let archive: Value = serde_json::from_str(include_str!(
            "../../../../../../docs/contracts/owner-beta-paper-operations-archive-v1.schema.json"
        ))
        .unwrap();
        const STAGE_KEYS: &[&str] = &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "sequence",
            "previous_sha256",
            "intent_sha256",
            "stage",
            "boot_id",
            "observed",
            "control_epoch",
            "artifact_sha256",
            "event_id",
            "error_code",
        ];
        const RECONCILIATION_KEYS: &[&str] = &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "observation_id",
            "control_epoch",
            "inventory_sha256",
            "observed_at",
            "database_sha256",
            "ledger_sha256",
            "pending_sha256",
            "outbox_sha256",
            "jobs_sha256",
            "unresolved_commits",
            "live_claims",
            "held_targets",
            "quarantined_deliveries",
        ];
        const RECOVERY_AUTHORITY_KEYS: &[&str] = &[
            "grant_id",
            "active_epoch",
            "active_process_sha256",
            "active_consent_id",
            "step",
            "expires_at",
        ];
        const RESULT_KEYS: &[&str] = &[
            "schema",
            "verb",
            "outcome",
            "event_id",
            "generation",
            "control_epoch",
            "artifact_sha256",
            "error_code",
        ];
        const ARCHIVE_KEYS: &[&str] = &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "profile",
            "control_epoch",
            "drain_receipt_sha256",
            "created_at",
            "release_manifest_sha256",
            "capability_sha256",
            "database_dump_sha256",
            "database_snapshot_sha256",
            "audit_sha256",
            "files_sha256",
            "classes",
        ];
        const AUDIT_KEYS: &[&str] = &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "control_epoch",
            "audit_sql_sha256",
            "migration_0068_sha256",
            "groups",
        ];
        const CAPABILITY_KEYS: &[&str] = &[
            "schema",
            "minimum",
            "code_commit",
            "release_manifest_sha256",
            "operations_schema_sha256",
            "archive_schema_sha256",
            "migration_0068_sha256",
            "audit_sql_sha256",
            "api_binary_sha256",
            "worker_binary_sha256",
            "native_binary_sha256",
        ];
        const FLOOR_KEYS: &[&str] = &["schema", "minimum"];
        let expected = [
            ("intent", INTENT_KEYS),
            ("clock", CLOCK_KEYS),
            ("stage", STAGE_KEYS),
            ("inventory", INVENTORY_KEYS),
            ("writer", WRITER_KEYS),
            ("reconciliation", RECONCILIATION_KEYS),
            ("drain", DRAIN_KEYS),
            ("restore", RESTORE_KEYS),
            ("recovery", RECOVERY_KEYS),
            ("recovery_authority", RECOVERY_AUTHORITY_KEYS),
            ("configure", CONFIGURE_KEYS),
            ("result", RESULT_KEYS),
        ];
        for (name, keys) in expected {
            let definition = &operations["$defs"][name];
            let required = definition["required"].as_array().unwrap();
            let properties = definition["properties"].as_object().unwrap();
            let mut expected_keys = keys.to_vec();
            let mut required_keys = required
                .iter()
                .map(Value::as_str)
                .map(Option::unwrap)
                .collect::<Vec<_>>();
            let mut property_keys = properties.keys().map(String::as_str).collect::<Vec<_>>();
            expected_keys.sort_unstable();
            required_keys.sort_unstable();
            property_keys.sort_unstable();
            assert_eq!(required_keys, expected_keys, "{name} required keys");
            assert_eq!(property_keys, expected_keys, "{name} closed properties");
        }
        let archive_expected = [
            ("archive", ARCHIVE_KEYS),
            ("audit", AUDIT_KEYS),
            ("capability", CAPABILITY_KEYS),
            ("floor", FLOOR_KEYS),
        ];
        for (name, keys) in archive_expected {
            let definition = &archive["$defs"][name];
            let required = definition["required"].as_array().unwrap();
            let properties = definition["properties"].as_object().unwrap();
            let mut expected_keys = keys.to_vec();
            let mut required_keys = required
                .iter()
                .map(Value::as_str)
                .map(Option::unwrap)
                .collect::<Vec<_>>();
            let mut property_keys = properties.keys().map(String::as_str).collect::<Vec<_>>();
            expected_keys.sort_unstable();
            required_keys.sort_unstable();
            property_keys.sort_unstable();
            assert_eq!(required_keys, expected_keys, "archive {name} required keys");
            assert_eq!(
                property_keys, expected_keys,
                "archive {name} closed properties"
            );
        }
    }
}
