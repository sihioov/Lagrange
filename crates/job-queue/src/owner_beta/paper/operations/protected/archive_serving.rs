//! Historical serving evidence stored as existing operation originals.
//! Archive validation never grants serving or recovery authority.
use super::*;

const SCHEMA: &str = "owner-beta-paper-serving-record-v1";
const PHASES: &[&str] = &[
    "intent",
    "enabled",
    "creating",
    "created",
    "paper-scheduler",
    "api-server",
    "started",
    "not-started",
    "stop-intent",
    "failed",
    "reconciled",
    "clearing-api-server",
    "cleared-api-server",
    "clearing-paper-scheduler",
    "cleared-paper-scheduler",
];
const SERVICES: &[&str] = &["api-server", "paper-scheduler"];

pub(super) fn is_serving(value: &Value) -> bool {
    value["schema"]
        .as_str()
        .is_some_and(|schema| schema.starts_with("owner-beta-paper-serving-"))
}

pub(super) fn retain(
    root: &wire::RootIntent,
    originals: &mut BTreeMap<Option<String>, (String, Value)>,
    hash: String,
    value: Value,
) -> Result<(), ErrorCode> {
    if originals.len() >= PHASES.len()
        || root.intent.purpose != "ENABLE"
        || root.intent.scope != "PAPER_STOP"
        || !fields(
            &value,
            &[
                "schema",
                "root_intent_sha256",
                "phase",
                "prior_sha256",
                "value",
            ],
        )
        || value["schema"] != SCHEMA
        || value["root_intent_sha256"] != root.sha256
        || !nullable_sha(&value["prior_sha256"])
        || !value["phase"]
            .as_str()
            .is_some_and(|phase| PHASES.contains(&phase))
        || !value["value"].is_object()
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let prior = value["prior_sha256"].as_str().map(str::to_owned);
    if originals.insert(prior, (hash, value)).is_some() {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

fn container_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn docker_generation(value: &Value) -> bool {
    fields(
        value,
        &["Id", "Image", "Created", "StartedAt", "RestartCount"],
    ) && container_id(&value["Id"])
        && sha(&value["Image"])
        && value["RestartCount"].as_u64().is_some()
        && ["Created", "StartedAt"].iter().all(|key| {
            value[*key]
                .as_str()
                .is_some_and(|stamp| (20..=40).contains(&stamp.len()) && stamp.ends_with('Z'))
        })
}

fn intent(root: &wire::RootIntent, value: &Value) -> bool {
    let original = &root.intent.value;
    fields(
        value,
        &[
            "schema",
            "enable_generation",
            "account_id",
            "expected_epoch",
            "release_manifest_sha256",
            "process_sha256",
            "prior_api_generation",
            "prior_daily_generations",
            "prior_proxy_generation",
            "installed_env_sha256",
        ],
    ) && value["schema"] == "owner-beta-paper-serving-intent-v1"
        && value["enable_generation"] == original["generation"]
        && value["account_id"] == original["account_id"]
        && value["expected_epoch"] == original["expected_epoch"]
        && value["release_manifest_sha256"] == original["release_manifest_sha256"]
        && value["process_sha256"] == original["artifact_sha256"]
        && sha(&value["process_sha256"])
        && sha(&value["installed_env_sha256"])
        && docker_generation(&value["prior_api_generation"])
        && docker_generation(&value["prior_proxy_generation"])
        && value["prior_daily_generations"]
            .as_object()
            .is_some_and(|services| {
                services.len() <= 8
                    && services
                        .values()
                        .all(|v| v.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 8))
            })
}

struct History<'a> {
    enable: &'a wire::RootIntent,
    activation: Option<&'a Value>,
    stop: Option<wire::RootIntent>,
    fenced: Option<&'a Value>,
}

impl History<'_> {
    fn verify(&self, phase: &str, value: &Value, known: &BTreeMap<String, Value>) -> bool {
        let root = &self.enable.intent;
        let generation = &root.value["generation"];
        let empty = Value::Null;
        let prior = |key: &str| known.get(key).unwrap_or(&empty);
        match phase {
            "intent" => known.is_empty() && intent(self.enable, value),
            "enabled" => {
                known.len() == 1
                    && fields(
                        value,
                        &[
                            "generation",
                            "account",
                            "epoch",
                            "event",
                            "manifest",
                            "process",
                        ],
                    )
                    && value["generation"] == *generation
                    && value["account"] == root.value["account_id"]
                    && value["manifest"] == root.value["release_manifest_sha256"]
                    && value["process"] == root.value["artifact_sha256"]
                    && self.activation.is_some_and(|row| {
                        value["event"] == row["id"] && value["epoch"] == row["control_epoch"]
                    })
            }
            "not-started" => {
                known.len() == 1
                    && self.stop.is_none()
                    && self.activation.is_none()
                    && fields(value, &["native_outcome", "native_cleanup"])
                    && value["native_outcome"] == "BLOCKED"
                    && value["native_cleanup"] == "COMPLETE"
            }
            "creating" => {
                known.len() == 2
                    && known.contains_key("enabled")
                    && fields(value, &["mounts_sha256"])
                    && fields(&value["mounts_sha256"], SERVICES)
                    && SERVICES
                        .iter()
                        .all(|service| sha(&value["mounts_sha256"][*service]))
            }
            "created" => {
                known.len() == 3
                    && known.contains_key("creating")
                    && fields(value, &["container_ids"])
                    && fields(&value["container_ids"], SERVICES)
                    && value["container_ids"]["api-server"]
                        != value["container_ids"]["paper-scheduler"]
                    && SERVICES.iter().all(|service| {
                        let id = &value["container_ids"][*service];
                        container_id(id) && *id != prior("intent")["prior_api_generation"]["Id"]
                    })
            }
            "api-server" | "paper-scheduler" => {
                let preceding_phases = if phase == "paper-scheduler" { 4 } else { 5 };
                known.len() == preceding_phases
                    && known.contains_key("created")
                    && (phase == "paper-scheduler" || known.contains_key("paper-scheduler"))
                    && fields(value, &["generation"])
                    && docker_generation(&value["generation"])
                    && value["generation"]["Id"] == prior("created")["container_ids"][phase]
            }
            "started" => {
                known.len() == 6
                    && fields(value, &["consumers"])
                    && fields(&value["consumers"], SERVICES)
                    && SERVICES
                        .iter()
                        .all(|service| value["consumers"][*service] == prior(service)["generation"])
            }
            "stop-intent" => {
                fields(value, &["generation", "intent_sha256"])
                    && self.stop.as_ref().is_some_and(|stop| {
                        value["generation"] == stop.intent.value["generation"]
                            && value["intent_sha256"] == stop.sha256
                    })
            }
            "failed" | "reconciled" => {
                fields(value, &["state", "fence", "enable_generation"])
                    && value["enable_generation"] == *generation
                    && ((phase == "failed"
                        && value["state"] == "UNKNOWN"
                        && value["fence"].is_null())
                        || (value["state"] == "FENCED_DRAINING"
                            && fields(&value["fence"], &["generation", "event_id", "epoch"])
                            && self.fenced.is_some_and(|row| {
                                value["fence"]["generation"] == row["generation"]
                                    && value["fence"]["event_id"] == row["fence_event_id"]
                                    && value["fence"]["epoch"] == row["fence_epoch"]
                            })))
            }
            _ => {
                let Some((action, service)) = phase.split_once('-') else {
                    return false;
                };
                let observed = &value["generation"];
                let id = &observed["Id"];
                (prior("failed")["state"] == "FENCED_DRAINING"
                    || prior("reconciled")["state"] == "FENCED_DRAINING")
                    && SERVICES.contains(&service)
                    && known.contains_key("creating")
                    && fields(value, &["service", "generation"])
                    && value["service"] == service
                    && docker_generation(observed)
                    && observed["RestartCount"] == 0
                    && observed["StartedAt"] == "0001-01-01T00:00:00Z"
                    && *id != prior("intent")["prior_api_generation"]["Id"]
                    && (!known.contains_key("created")
                        || *id == prior("created")["container_ids"][service])
                    && (action == "clearing" || prior(&format!("clearing-{service}")) == value)
            }
        }
    }
}

impl Domains {
    pub(super) fn finish_serving(
        &self,
        root: &wire::RootIntent,
        generation: &str,
        mut originals: BTreeMap<Option<String>, (String, Value)>,
        index: &JournalIndex<'_>,
    ) -> Result<(), ErrorCode> {
        let key = index.serving_stops.get(generation);
        if originals.is_empty() && key.is_none() {
            return Ok(());
        }
        let conflict = ErrorCode::IdentityConflict;
        let epoch = root.fence_epoch().map_err(|_| conflict)?;
        if root.intent.expected_epoch.checked_add(1) != Some(epoch)
            || root.intent.purpose != "ENABLE"
            || root.intent.scope != "PAPER_STOP"
        {
            return Err(conflict);
        }
        let stop = key
            .map(|generation| {
                let path = format!("{JOURNAL}/operations/{generation}/intent.json");
                let stop = historical_root(self.file(&path)?)?;
                if stop.intent.expected_epoch != epoch
                    || stop.intent.local_epoch_floor != epoch
                    || stop.intent.account_id != root.intent.account_id
                    || stop.intent.installation_id != root.intent.installation_id
                    || stop.intent.release_manifest_sha256 != root.intent.release_manifest_sha256
                {
                    return Err(conflict);
                }
                Ok(stop)
            })
            .transpose()?;
        let fenced = key
            .and_then(|id| index.operations.get(id))
            .map(|row| &index.rows[*row].1);
        let mut activation = None;
        for (table, row) in &index.rows {
            self.check_time()
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if *table == RECEIPT_TABLES[2]
                && row["request_key"] == root.intent.request_key
                && row["account_id"] == root.intent.value["account_id"]
            {
                if activation.is_some()
                    || row["kind"] != "ENABLED"
                    || row["control_epoch"] != epoch
                    || !uuid(&row["id"])
                    || !sha(&row["request_sha256"])
                    || !sha(&row["result_sha256"])
                    || row["payload"]["process_binding_sha256"]
                        != root.intent.value["artifact_sha256"]
                {
                    return Err(conflict);
                }
                activation = Some(row);
            }
        }
        if key.is_some() && activation.is_none() {
            return Err(conflict);
        }
        if let (Some(stop), Some(row)) = (stop.as_ref(), fenced) {
            let generation = stop.intent.generation;
            let stage = self.file(&format!("{JOURNAL}/operations/{generation}/00000001.json"))?;
            let stage = wire::parse_canonical(stage).map_err(|_| ErrorCode::WireInvalid)?;
            if row["prior_state"] != "ENABLED"
                || !sha(&row["request_sha256"])
                || !sha(&row["result_sha256"])
                || stage["stage"] != "FENCED"
                || stage["control_epoch"] != row["fence_epoch"]
                || stage["event_id"] != row["fence_event_id"]
                || stage["intent_sha256"] != stop.sha256
            {
                return Err(conflict);
            }
        }
        let history = History {
            enable: root,
            activation,
            stop,
            fenced,
        };
        let mut known = BTreeMap::new();
        let mut tail = None;
        while let Some((hash, record)) = originals.remove(&tail) {
            let phase = record["phase"].as_str().ok_or(conflict)?;
            let cancelled = known.contains_key("not-started");
            if known.contains_key(phase)
                || (phase == "intent") != known.is_empty()
                // An fsync/ack error may leave both the cancelled receipt
                // and the subsequent failure diagnosis durably present.
                || (cancelled && (phase != "failed" || record["value"]["state"] != "UNKNOWN"))
                || !history.verify(phase, &record["value"], &known)
            {
                return Err(conflict);
            }
            known.insert(phase.to_owned(), record["value"].clone());
            tail = Some(hash);
        }
        if !originals.is_empty()
            || !known.contains_key("intent")
            || (fenced.is_some()
                && !["failed", "reconciled"].iter().any(|phase| {
                    known
                        .get(*phase)
                        .is_some_and(|value| value["state"] == "FENCED_DRAINING")
                }))
        {
            return Err(conflict);
        }
        Ok(())
    }
}
