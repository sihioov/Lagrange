use serde_json::Value;
use uuid::Uuid;

use super::Verb;

pub(super) const MAX_WIRE_BYTES: usize = 65_536;
pub(super) const MAX_DEPTH: usize = 16;
pub(super) const MAX_PAGE_BYTES: usize = 262_144;
pub(super) const MAX_PAGES: usize = 512;

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
    if bytes.is_empty() || bytes.len() > MAX_WIRE_BYTES {
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

#[cfg(test)]
mod tests {
    use super::{
        CLOCK_KEYS, CONFIGURE_KEYS, DRAIN_KEYS, INTENT_KEYS, INVENTORY_KEYS, RECOVERY_KEYS,
        RESTORE_KEYS, WRITER_KEYS, parse_artifact, parse_canonical, parse_capability,
        parse_configure, parse_intent, parse_process_binding,
    };
    use crate::owner_beta::paper::operations::Verb;
    use serde_json::{Value, json};

    fn canonical(v: serde_json::Value) -> Vec<u8> {
        crate::owner_beta::paper::source_evidence::canonical(&v).unwrap()
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
            Verb::Configure | Verb::Inspect => "request",
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
