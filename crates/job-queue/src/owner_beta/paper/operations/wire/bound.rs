//! Closed, untrusted §10 originals. None of these values proves live custody,
//! a committed fence, stopped writers, eligibility, or recovery authority.
use super::*;

pub(in super::super) const MAX_DISPATCH_BYTES: usize = 2 * MAX_EXACT_BYTES;

pub(in super::super) struct RootIntent {
    pub intent: Intent,
    pub canonical: String,
    pub sha256: String,
}

impl RootIntent {
    pub fn fence_epoch(&self) -> Result<i64, ()> {
        self.intent
            .expected_epoch
            .max(self.intent.local_epoch_floor)
            .checked_add(1)
            .ok_or(())
    }

    pub fn maintenance(&self) -> bool {
        matches!(self.intent.purpose.as_str(), "STOP" | "BACKUP" | "RESTORE")
    }
}

pub(in super::super) struct ActionBinding {
    pub canonical: String,
    pub sha256: String,
    pub verb: Verb,
    pub request_key: String,
    pub expected_epoch: i64,
    pub artifact_sha256: Option<String>,
    pub stage_sha256: Option<String>,
    pub sequence: i64,
    pub previous_binding_sha256: Option<String>,
}

fn optional_digest(value: &Value, key: &str) -> Result<Option<String>, ()> {
    match value.get(key).ok_or(())? {
        Value::Null => Ok(None),
        Value::String(hash) if digest(hash) => Ok(Some(hash.clone())),
        _ => Err(()),
    }
}

pub(in super::super) fn parse_root_intent(bytes: &[u8]) -> Result<RootIntent, ()> {
    let value = parse_canonical(bytes)?;
    exact_keys(&value, INTENT_KEYS)?;
    if text(&value, "schema")? != "owner-beta-paper-operation-intent-v1" {
        return Err(());
    }
    let installation_id = parse_uuid(text(&value, "installation_id")?)?;
    let generation = parse_uuid(text(&value, "generation")?)?;
    let account_id = optional_uuid(&value, "account_id")?;
    let predecessor = optional_uuid(&value, "predecessor_generation")?;
    let resume = optional_uuid(&value, "resume_target_id")?;
    let key = text(&value, "request_key")?;
    request_key(key)?;
    let purpose = text(&value, "purpose")?;
    let scope = text(&value, "scope")?;
    let epoch = i64_value(&value, "expected_epoch")?;
    let floor = i64_value(&value, "local_epoch_floor")?;
    let release = text(&value, "release_manifest_sha256")?;
    let artifact = optional_digest(&value, "artifact_sha256")?;
    let maintenance = matches!(purpose, "STOP" | "BACKUP" | "RESTORE");
    if !digest(release)
        || predecessor == Some(generation)
        || !matches!(
            (purpose, scope),
            ("STOP", "PAPER_STOP")
                | ("BACKUP" | "RESTORE", "COMBINED_BACKUP")
                | (
                    "CONFIGURE" | "ENABLE" | "RECOVERY" | "INSPECT",
                    "PAPER_STOP"
                )
        )
        || (purpose != "RESTORE" && floor != epoch)
        || (!maintenance && predecessor.is_some())
        || (purpose == "CONFIGURE" && (account_id.is_some() || epoch != 0))
        || (!matches!(purpose, "CONFIGURE" | "INSPECT") && account_id.is_none())
        || (purpose == "INSPECT" && account_id.is_none() && epoch != 0)
        || (!matches!(purpose, "ENABLE" | "RECOVERY") && resume.is_some())
        || (purpose == "RECOVERY" && resume.is_none())
        || (matches!(purpose, "STOP" | "BACKUP") && artifact.is_some())
        || (matches!(purpose, "CONFIGURE" | "ENABLE" | "RECOVERY" | "RESTORE")
            && artifact.is_none())
        || (purpose == "RESTORE" && predecessor.is_none())
    {
        return Err(());
    }
    if (maintenance && key != format!("{generation}:begin"))
        || (purpose == "ENABLE" && key != format!("{generation}:enable"))
        || (purpose == "RECOVERY" && key != format!("{generation}:recover"))
    {
        return Err(());
    }
    let intent = Intent {
        installation_id,
        generation,
        account_id,
        request_key: key.to_owned(),
        purpose: purpose.to_owned(),
        scope: scope.to_owned(),
        expected_epoch: epoch,
        local_epoch_floor: floor,
        release_manifest_sha256: release.to_owned(),
        artifact_sha256: artifact,
        value,
    };
    let root = RootIntent {
        intent,
        canonical: String::from_utf8(bytes.to_vec()).map_err(|_| ())?,
        sha256: super::super::super::source_evidence::digest(bytes),
    };
    // Proposed arithmetic is validated, but never treated as a committed fence.
    if maintenance {
        root.fence_epoch()?.checked_add(1).ok_or(())?;
    }
    Ok(root)
}

pub(in super::super) fn parse_action(bytes: &[u8], root: &RootIntent) -> Result<ActionBinding, ()> {
    let value = parse_canonical(bytes)?;
    exact_keys(
        &value,
        &[
            "schema",
            "installation_id",
            "generation",
            "root_intent_sha256",
            "sequence",
            "previous_binding_sha256",
            "verb",
            "request_key",
            "expected_epoch",
            "artifact_sha256",
            "stage_sha256",
        ],
    )?;
    let r = &root.intent;
    let verb = Verb::parse(text(&value, "verb")?).ok_or(())?;
    let sequence = i64_value(&value, "sequence")?;
    let previous = optional_digest(&value, "previous_binding_sha256")?;
    let artifact = optional_digest(&value, "artifact_sha256")?;
    let stage = optional_digest(&value, "stage_sha256")?;
    let epoch = i64_value(&value, "expected_epoch")?;
    let key = text(&value, "request_key")?;
    request_key(key)?;
    if text(&value, "schema")? != "owner-beta-paper-action-binding-v1"
        || text(&value, "installation_id")? != r.installation_id.to_string()
        || text(&value, "generation")? != r.generation.to_string()
        || text(&value, "root_intent_sha256")? != root.sha256
        || (sequence == 0) != previous.is_none()
        || !matches!(
            (r.purpose.as_str(), verb),
            ("CONFIGURE", Verb::Configure)
                | ("ENABLE", Verb::Enable)
                | ("RECOVERY", Verb::Recover)
                | ("STOP" | "BACKUP", Verb::Begin | Verb::Reconcile | Verb::Off)
                | ("RESTORE", Verb::Begin | Verb::Reconcile | Verb::RestoreOff)
        )
    {
        return Err(());
    }
    if root.maintenance() {
        if stage.is_none()
            || (verb == Verb::Begin) != (sequence == 0)
            || (verb == Verb::Begin) != artifact.is_none()
            || epoch
                != if verb == Verb::Begin {
                    r.expected_epoch
                } else {
                    root.fence_epoch()?
                }
        {
            return Err(());
        }
    } else if sequence != 0
        || stage.is_some()
        || artifact != r.artifact_sha256
        || epoch != r.expected_epoch
    {
        return Err(());
    }
    if verb == Verb::Reconcile {
        let suffix = key
            .strip_prefix(&format!("{}:reconcile:", r.generation))
            .ok_or(())?;
        if suffix.is_empty()
            || suffix.len() > 18
            || suffix.starts_with('0')
            || !suffix.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(());
        }
        // Before the terminal OFF every action after BEGIN is a reconcile.
        if suffix.parse::<i64>().map_err(|_| ())? != sequence {
            return Err(());
        }
    } else if key
        != if verb == Verb::Configure {
            r.request_key.clone()
        } else {
            format!("{}:{}", r.generation, verb.as_str())
        }
    {
        return Err(());
    }
    Ok(ActionBinding {
        canonical: String::from_utf8(bytes.to_vec()).map_err(|_| ())?,
        sha256: super::super::super::source_evidence::digest(bytes),
        verb,
        request_key: key.to_owned(),
        expected_epoch: epoch,
        artifact_sha256: artifact,
        stage_sha256: stage,
        sequence,
        previous_binding_sha256: previous,
    })
}

pub(in super::super) fn parse_bound_artifact(
    bytes: &[u8],
    root: &RootIntent,
    action: &ActionBinding,
) -> Result<Artifact, ()> {
    let sha256 = super::super::super::source_evidence::digest(bytes);
    if action.artifact_sha256.as_deref() != Some(sha256.as_str()) {
        return Err(());
    }
    let r = &root.intent;
    let value = match action.verb {
        Verb::Configure => parse_configure(bytes)?,
        Verb::Enable => return parse_process_binding(bytes, r),
        Verb::Recover => {
            let artifact = parse_artifact(bytes, "owner-beta-paper-recovery-v2", r)?;
            let projection = serde_json::json!({
                "effective_session":artifact.value["effective_session"],
                "pending_target_id":artifact.value["pending_target_id"], "step":artifact.value["step"] });
            let bytes =
                super::super::super::source_evidence::canonical(&projection).map_err(|_| ())?;
            if artifact.value["decision_sha256"]
                != super::super::super::source_evidence::digest(&bytes)
            {
                return Err(());
            }
            return Ok(artifact);
        }
        Verb::Reconcile => {
            // The inventory's identity is the root, but its hash belongs to A.
            // Reuse shape validation only; do not fabricate an invocation R.
            let value = parse_canonical(bytes)?;
            validate_inventory_bound(&value, root)?;
            value
        }
        Verb::Off | Verb::RestoreOff => {
            let value = parse_canonical(bytes)?;
            validate_transition_mode(
                if action.verb == Verb::Off {
                    "OFF"
                } else {
                    "RESTORED_OFF"
                },
                &value,
                Some(root),
            )?;
            for key in [
                "installation_id",
                "generation",
                "account_id",
                "scope",
                "release_manifest_sha256",
            ] {
                if value[key] != r.value[key] {
                    return Err(());
                }
            }
            if text(&value, "intent_sha256")? != root.sha256
                || text(&value, "request_key")? != action.request_key
                || value["stage_sha256"].as_str() != action.stage_sha256.as_deref()
                || action.expected_epoch != root.fence_epoch()?
            {
                return Err(());
            }
            if action.verb == Verb::RestoreOff {
                let source_epoch = i64_value(&value, "source_epoch")?;
                if text(&value, "source_installation_id")? == r.installation_id.to_string()
                    || value["source_generation"] != r.value["predecessor_generation"]
                    || value["archive_sha256"] != r.value["artifact_sha256"]
                    || i64_value(&value, "destination_epoch_floor")? != r.local_epoch_floor
                    || source_epoch.checked_add(1).ok_or(())? > r.local_epoch_floor
                    || value["source_audit_sha256"] != value["restored_audit_sha256"]
                {
                    return Err(());
                }
            }
            value
        }
        _ => return Err(()),
    };
    Ok(Artifact { value, sha256 })
}

pub(in super::super) fn validate_inventory_bound(
    value: &Value,
    root: &RootIntent,
) -> Result<(), ()> {
    exact_keys(value, INVENTORY_KEYS)?;
    if text(value, "schema")? != "owner-beta-paper-writer-inventory-v1" {
        return Err(());
    }
    for key in [
        "installation_id",
        "generation",
        "account_id",
        "scope",
        "release_manifest_sha256",
    ] {
        if value[key] != root.intent.value[key] {
            return Err(());
        }
    }
    if i64_value(value, "control_epoch")? != root.fence_epoch()? {
        return Err(());
    }
    for key in ["fence_event_id", "boot_id"] {
        parse_uuid(text(value, key)?)?;
    }
    for key in ["launch_fence_sha256", "backend_observation_sha256"] {
        if !digest(text(value, key)?) {
            return Err(());
        }
    }
    validate_clock(&value["observed"])?;
    let writers = value["writers"].as_array().ok_or(())?;
    if writers.len() > 64 {
        return Err(());
    }
    let mut previous = None;
    for writer in writers {
        exact_keys(writer, WRITER_KEYS)?;
        let class = text(writer, "writer_class")?;
        let kind = text(writer, "kind")?;
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
        .contains(&class)
            || !["CONTAINER", "PROCESS", "UNIT"].contains(&kind)
            || !["EXITED_REAPED", "ABSENT_AT_FENCE"].contains(&text(writer, "terminal")?)
        {
            return Err(());
        }
        for key in [
            "identity_sha256",
            "observation_sha256",
            "backend_set_sha256",
        ] {
            if !digest(text(writer, key)?) {
                return Err(());
            }
        }
        let order = (text(writer, "identity_sha256")?, class, kind);
        if previous.is_some_and(|p| p >= order) {
            return Err(());
        }
        previous = Some(order);
    }
    Ok(())
}

pub(in super::super) fn parse_exact_bound(
    bytes: &[u8],
    root: &RootIntent,
    action: &ActionBinding,
) -> Result<ExactRequest, ()> {
    let request = ExactRequest::parse_mode(bytes, Some(root))?;
    if request.original_verb()? != action.verb
        || optional_uuid(&request.value, "account_id")? != root.intent.account_id
        || text(&request.value["args"], "request_key")? != action.request_key
        || (action.verb != Verb::Configure
            && i64_value(&request.value["args"], "expected_epoch")? != action.expected_epoch)
    {
        return Err(());
    }
    if action.verb == Verb::Begin
        && text(&request.value["args"], "intent_canonical")? != root.canonical
    {
        return Err(());
    }
    Ok(request)
}

pub(in super::super) fn correlate_bound(
    root: &RootIntent,
    action: &ActionBinding,
    artifact: Option<&Artifact>,
    request: &ExactRequest,
) -> Result<(), ()> {
    // Reparse to make the correlation safe for requests constructed by legacy
    // producers as well as for readback input. Preserve the supplied Q bytes.
    parse_exact_bound(request.canonical.as_bytes(), root, action)?;
    if action.artifact_sha256.as_deref() != artifact.map(|a| a.sha256.as_str()) {
        return Err(());
    }
    if let Some(artifact) = artifact {
        let bytes =
            super::super::super::source_evidence::canonical(&artifact.value).map_err(|_| ())?;
        if parse_bound_artifact(&bytes, root, action)?.sha256 != artifact.sha256 {
            return Err(());
        }
    }
    let args = &request.value["args"];
    match action.verb {
        Verb::Configure | Verb::Enable | Verb::Recover => request.correlate(&root.intent, artifact),
        Verb::Begin => {
            if artifact.is_none() {
                Ok(())
            } else {
                Err(())
            }
        }
        Verb::Reconcile => {
            if args["generation"] != root.intent.generation.to_string()
                || args["inventory_sha256"].as_str() != action.artifact_sha256.as_deref()
            {
                return Err(());
            }
            Ok(())
        }
        Verb::Off | Verb::RestoreOff => {
            let artifact = artifact.ok_or(())?;
            let bytes =
                super::super::super::source_evidence::canonical(&artifact.value).map_err(|_| ())?;
            if text(args, "evidence_canonical")?.as_bytes() != bytes {
                return Err(());
            }
            parse_bound_artifact(&bytes, root, action)?;
            Ok(())
        }
        _ => Err(()),
    }
}

/// Complete original index; it is data, never a dispatch/retry permit.
pub(in super::super) struct BoundDispatch {
    pub canonical: String,
    pub sha256: String,
    pub root: RootIntent,
    pub action: ActionBinding,
    pub artifact: Option<Artifact>,
    pub request: ExactRequest,
}

impl BoundDispatch {
    pub fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let value = parse_canonical_limit(bytes, MAX_DISPATCH_BYTES)?;
        exact_keys(
            &value,
            &[
                "schema",
                "root_intent_canonical",
                "root_intent_sha256",
                "action_canonical",
                "action_sha256",
                "artifact_canonical",
                "artifact_sha256",
                "sql_request_canonical",
                "sql_request_sha256",
            ],
        )?;
        if text(&value, "schema")? != "owner-beta-paper-native-dispatch-v2" {
            return Err(());
        }
        let root = parse_root_intent(text(&value, "root_intent_canonical")?.as_bytes())?;
        let action = parse_action(text(&value, "action_canonical")?.as_bytes(), &root)?;
        let artifact = match &value["artifact_canonical"] {
            Value::Null => None,
            Value::String(bytes) => Some(parse_bound_artifact(bytes.as_bytes(), &root, &action)?),
            _ => return Err(()),
        };
        let request = parse_exact_bound(
            text(&value, "sql_request_canonical")?.as_bytes(),
            &root,
            &action,
        )?;
        if text(&value, "root_intent_sha256")? != root.sha256
            || text(&value, "action_sha256")? != action.sha256
            || optional_digest(&value, "artifact_sha256")?.as_deref()
                != artifact.as_ref().map(|a| a.sha256.as_str())
            || text(&value, "sql_request_sha256")? != request.sha256
        {
            return Err(());
        }
        correlate_bound(&root, &action, artifact.as_ref(), &request)?;
        Ok(Self {
            canonical: String::from_utf8(bytes.to_vec()).map_err(|_| ())?,
            sha256: super::super::super::source_evidence::digest(bytes),
            root,
            action,
            artifact,
            request,
        })
    }

    pub fn build(
        root: &RootIntent,
        action: &ActionBinding,
        artifact: Option<&Artifact>,
        request: &ExactRequest,
    ) -> Result<Self, ()> {
        correlate_bound(root, action, artifact, request)?;
        let artifact_canonical = artifact
            .map(|a| {
                super::super::super::source_evidence::canonical(&a.value)
                    .map_err(|_| ())
                    .and_then(|b| String::from_utf8(b).map_err(|_| ()))
            })
            .transpose()?;
        let bytes = super::super::super::source_evidence::canonical(&serde_json::json!({
            "schema":"owner-beta-paper-native-dispatch-v2", "root_intent_canonical":root.canonical,
            "root_intent_sha256":root.sha256, "action_canonical":action.canonical, "action_sha256":action.sha256,
            "artifact_canonical":artifact_canonical, "artifact_sha256":artifact.map(|a| &a.sha256),
            "sql_request_canonical":request.canonical, "sql_request_sha256":request.sha256,
        })).map_err(|_| ())?;
        Self::parse(&bytes)
    }
}

#[cfg(test)]
mod tests;
