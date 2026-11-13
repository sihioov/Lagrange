//! Descriptive sibling originals, consumed only by the mandatory archive finish.
//! No source anchor can become a destination capability or a new boot proof.
use super::*;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct NativeResult {
    schema: String,
    verb: String,
    outcome: String,
    // Value deliberately requires explicit null, unlike Option's missing default.
    event_id: Value,
    generation: Value,
    control_epoch: Value,
    artifact_sha256: Value,
    error_code: String,
}
impl NativeResult {
    fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let raw = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        if raw.len() > 65_536 || !raw.is_ascii() || raw.contains(&b'\n') || raw.contains(&b'\r') {
            return Err(());
        }
        let r: Self = serde_json::from_slice(raw).map_err(|_| ())?;
        let verb = super::super::super::Verb::parse(&r.verb).ok_or(())?;
        if r.schema != verb.result_schema()
            || serde_json::to_vec(&r).map_err(|_| ())? != raw
            || !(r.event_id.is_null() || uuid(&r.event_id))
            || !(r.generation.is_null() || uuid(&r.generation))
            || !(r.control_epoch.is_null() || r.control_epoch.as_i64().is_some_and(|n| n >= 0))
            || !nullable_sha(&r.artifact_sha256)
            || ![
                "NONE",
                "WIRE_INVALID",
                "IDENTITY_CONFLICT",
                "STALE_EPOCH",
                "CLOCK_INVALID",
                "WRITER_UNKNOWN",
                "COMMIT_UNKNOWN",
                "READER_INCOMPATIBLE",
                "RECOVERY_AUTHORITY_UNAVAILABLE",
                "EVIDENCE_UNAVAILABLE",
                "INTERNAL_FAILURE",
            ]
            .contains(&r.error_code.as_str())
            || match r.outcome.as_str() {
                "OK" | "REPLAY" => r.error_code != "NONE",
                "COMMIT_UNKNOWN" => r.error_code != "COMMIT_UNKNOWN",
                "BLOCKED" => r.error_code == "NONE",
                _ => true,
            }
        {
            return Err(());
        }
        Ok(r)
    }
    fn success(&self) -> bool {
        matches!(self.outcome.as_str(), "OK" | "REPLAY")
    }
}

fn selection_equal(a: &wire::ResetSelection, b: &wire::ResetSelection) -> bool {
    a.installation_id == b.installation_id
        && a.reset == b.reset
        && a.release_manifest_sha256 == b.release_manifest_sha256
        && a.cohort_sha256 == b.cohort_sha256
}
impl Domains {
    pub(super) fn finish_lifecycle(&self) -> Result<(), SourceClosureError> {
        self.lifecycle().map_err(|e| {
            // Preserve the original shared deadline rather than resetting it.
            self.check_time().err().unwrap_or(e)
        })
    }
    fn lifecycle(&self) -> Result<(), SourceClosureError> {
        let bad = || SourceClosureError::Journal;
        let paths = self
            .members
            .keys()
            .filter(|p| p.starts_with("paper_evidence/lifecycle/"));
        if !self.members.contains_key(LIFECYCLE) {
            if paths.count() != 0 {
                return Err(bad());
            }
            // A bound modern ENABLE cannot erase its causal sibling originals.
            for (path, member) in &self.members {
                self.check_time()?;
                if path.starts_with("paper_operations/operations/")
                    && path.ends_with("/output/dispatch.json")
                {
                    let d = wire::BoundDispatch::parse(member.bytes.as_deref().ok_or_else(bad)?)
                        .map_err(|_| bad())?;
                    if d.request.value["args"]["transition"] == "ENABLE" {
                        return Err(bad());
                    }
                }
            }
            return Ok(());
        }
        if !self.directory(LIFECYCLE) {
            return Err(bad());
        }
        for path in paths {
            self.check_time()?;
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if !self.directory(prefix) {
                    return Err(bad());
                }
                if prefix == LIFECYCLE {
                    break;
                }
                parent = prefix;
            }
        }
        let roots = ["queries", "selections", "anchors"];
        if self.children(LIFECYCLE).any(|n| !roots.contains(&n))
            || roots
                .iter()
                .any(|n| !self.directory(&format!("{LIFECYCLE}/{n}")))
        {
            return Err(bad());
        }
        let installation = self
            .members
            .get("paper_operations/installation-id")
            .and_then(|m| m.bytes.as_deref())
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| s.strip_suffix('\n'))
            .unwrap_or("");
        let mut index = self
            .journal_index("paper_operations/callers", installation)
            .map_err(|_| bad())?;
        // This scratch reservation covers parsed trees, indices, strings and source
        // ARM values in addition to the already charged original byte owners.
        for (path, member) in &self.members {
            if path.starts_with("paper_evidence/") {
                self.check_time()?;
                let cost = member
                    .bytes
                    .as_ref()
                    .map_or(0, Vec::len)
                    .checked_mul(16)
                    .and_then(|n| n.checked_add(path.len()))
                    .ok_or(SourceClosureError::Limit)?;
                index
                    .charge(self, cost)
                    .map_err(|_| SourceClosureError::Limit)?;
            }
        }
        for ((table, _), raw) in &self.rows {
            if [
                "owner_beta_paper_control",
                "owner_beta_paper_source_runs",
                "owner_beta_paper_source_slots",
                "owner_beta_paper_source_admissions",
            ]
            .contains(&table.as_str())
            {
                index
                    .charge(
                        self,
                        raw.len().checked_mul(16).ok_or(SourceClosureError::Limit)?,
                    )
                    .map_err(|_| SourceClosureError::Limit)?;
            }
        }
        let mut queries = Vec::new();
        let query_dir = format!("{LIFECYCLE}/queries");
        for name in self.children(&query_dir) {
            self.check_time()?;
            if !uuid(&Value::String(name.into())) {
                return Err(bad());
            }
            let dir = format!("{query_dir}/{name}");
            self.lifecycle_query(&dir, installation, &index)
                .map_err(|_| bad())?;
            queries.push(dir);
        }
        let mut selections = Vec::new();
        for name in self.children(&format!("{LIFECYCLE}/selections")) {
            self.check_time()?;
            let path = format!("{LIFECYCLE}/selections/{name}");
            let bytes = self.file(&path).map_err(|_| bad())?;
            if named_digest(name).as_deref() != Some(digest_bytes(bytes).as_str()) {
                return Err(bad());
            }
            let selection = wire::ResetSelection::parse(bytes).map_err(|_| bad())?;
            if selection.installation_id.to_string() != installation {
                return Err(bad());
            }
            self.lifecycle_reset(&selection.reset, &index)
                .map_err(|_| bad())?;
            // The original RESET query, not a selection's assertion, describes
            // where the boundary was read. It can be older than latest BACKUP OFF.
            let mut found = false;
            for dir in &queries {
                self.check_time()?;
                if let (Ok(r), Ok(a), Ok(q), Ok(root)) = (
                    self.file(&format!("{dir}/native-result.json")),
                    self.file(&format!("{dir}/artifact.json")),
                    self.file(&format!("{dir}/sql-request.json")),
                    self.file(&format!("{dir}/request.json")),
                ) {
                    let result = NativeResult::parse(r).map_err(|_| bad())?;
                    let q = wire::ExactRequest::parse(q);
                    let root = wire::parse_root_intent(root).map_err(|_| bad())?;
                    if result.verb == "readback"
                        && result.outcome == "OK"
                        && q.is_ok_and(|q| q.value["kind"] == "RESET")
                    {
                        let v = wire::parse_canonical_limit(a, wire::MAX_EXACT_BYTES)
                            .map_err(|_| bad())?;
                        if wire::ResetBoundary::parse(&v["reset"]).ok().as_ref()
                            == Some(&selection.reset)
                            && root.intent.release_manifest_sha256
                                == selection.release_manifest_sha256
                        {
                            found = true;
                        }
                    }
                }
            }
            if !found {
                return Err(bad());
            }
            selections.push(selection);
        }
        let mut anchor_boots = BTreeSet::new();
        for name in self.children(&format!("{LIFECYCLE}/anchors")) {
            self.check_time()?;
            let bytes = self
                .file(&format!("{LIFECYCLE}/anchors/{name}"))
                .map_err(|_| bad())?;
            if named_digest(name).as_deref() != Some(digest_bytes(bytes).as_str()) {
                return Err(bad());
            }
            let anchor = wire::ResetAnchor::parse(bytes).map_err(|_| bad())?;
            let identity = (
                anchor.selection.reset.event_id,
                anchor.selection.release_manifest_sha256.clone(),
                anchor.selection.cohort_sha256.clone(),
                anchor.boot_id,
            );
            if !anchor_boots.insert(identity) {
                return Err(bad());
            }
            if !selections
                .iter()
                .any(|s| selection_equal(s, &anchor.selection))
            {
                return Err(bad());
            }
            let mut produced = false;
            for dir in &queries {
                self.check_time()?;
                if let (Ok(raw), Ok(a)) = (
                    self.file(&format!("{dir}/native-result.json")),
                    self.file(&format!("{dir}/artifact.json")),
                ) {
                    let r = NativeResult::parse(raw).map_err(|_| bad())?;
                    if a == bytes && r.verb == "observe-reset" && r.outcome == "OK" {
                        produced = true;
                    }
                }
            }
            if !produced {
                return Err(bad());
            }
        }
        // Modern successful ENABLEs require the causal reset/boot originals even
        // if their sibling query was interrupted before stdout was copied.
        for (_, row) in &index.rows {
            self.check_time()?;
            if let Some(q) = row["request_canonical"].as_str()
                && let Some(dispatch) = index.requests.get(&digest_bytes(q.as_bytes()))
            {
                let d = wire::BoundDispatch::parse(dispatch).map_err(|_| bad())?;
                if d.request.value["args"]["transition"] == "ENABLE" {
                    self.lifecycle_enable(&d.root, &d.request, &selections)
                        .map_err(|_| bad())?;
                }
            }
        }
        Ok(())
    }

    fn lifecycle_reset(
        &self,
        reset: &wire::ResetBoundary,
        index: &JournalIndex<'_>,
    ) -> Result<(), ()> {
        let mut matches = 0;
        for (table, row) in &index.rows {
            self.check_time().map_err(|_| ())?;
            if *table != "owner_beta_paper_events" || row["id"] != reset.event_id.to_string() {
                continue;
            }
            let kind = match reset.kind {
                wire::ResetKind::Configured => "CONFIGURED",
                wire::ResetKind::Off => "OFF",
                wire::ResetKind::RestoredOff => "RESTORED_OFF",
            };
            let at = row["occurred_at"].as_str().ok_or(())?;
            let time = chrono::DateTime::parse_from_rfc3339(at).map_err(|_| ())?;
            if time.to_rfc3339_opts(chrono::SecondsFormat::Micros, true) != at
                || row["account_id"] != reset.account_id.to_string()
                || row["kind"] != kind
                || row["control_epoch"] != reset.control_epoch
                || time.timestamp_micros() != reset.occurred_at_us
                || row["payload_sha256"] != reset.payload_sha256
            {
                return Err(());
            }
            matches += 1;
        }
        if matches == 1 { Ok(()) } else { Err(()) }
    }

    fn lifecycle_readback(
        &self,
        bytes: &[u8],
        q: &wire::ExactRequest,
        index: &JournalIndex<'_>,
    ) -> Result<Value, ()> {
        let value = wire::parse_canonical_limit(bytes, wire::MAX_EXACT_BYTES)?;
        let read = wire::ExactReadback::parse(value, q)?;
        if read.value["status"] == "RESET" {
            self.lifecycle_reset(&wire::ResetBoundary::parse(&read.value["reset"])?, index)?;
        } else if read.value["status"] == "COMMITTED" {
            let raw = index.requests.get(&q.sha256).copied().ok_or(())?;
            let dispatch = wire::BoundDispatch::parse(raw)?;
            if dispatch.request.canonical != q.canonical {
                return Err(());
            }
            self.check_committed_readback(&read, &dispatch, index)
                .map_err(|_| ())?;
        }
        self.check_readback_current_epoch(&read.value)
            .map_err(|_| ())?;
        let read = read.value;
        Ok(read)
    }

    fn lifecycle_query(
        &self,
        dir: &str,
        installation: &str,
        index: &JournalIndex<'_>,
    ) -> Result<(), ()> {
        if !self.directory(dir) {
            return Err(());
        }
        let allowed = [
            "request.json",
            "evidence.json",
            "sql-request.json",
            "action-binding.json",
            "output",
            "native-result.json",
            "artifact.json",
        ];
        if self.children(dir).any(|n| !allowed.contains(&n)) {
            return Err(());
        }
        let optional = |leaf: &str| -> Result<Option<&[u8]>, ()> {
            let path = format!("{dir}/{leaf}");
            if self.members.contains_key(&path) {
                self.file(&path).map(Some).map_err(|_| ())
            } else {
                Ok(None)
            }
        };
        let Some(rbytes) = optional("request.json")? else {
            return if self.children(dir).next().is_none() {
                Ok(())
            } else {
                Err(())
            };
        };
        let root = wire::parse_root_intent(rbytes)?;
        if root.intent.installation_id.to_string() != installation {
            return Err(());
        }
        let e = optional("evidence.json")?;
        let qbytes = optional("sql-request.json")?;
        let a = optional("action-binding.json")?;
        let result = optional("native-result.json")?
            .map(NativeResult::parse)
            .transpose()?;
        let artifact = optional("artifact.json")?;
        let output = format!("{dir}/output");
        let has_output = self.members.contains_key(&output);
        if has_output && !self.directory(&output) {
            return Err(());
        }
        if (result.is_some() || artifact.is_some()) && !has_output {
            return Err(());
        }
        let bound = root.intent.purpose != "INSPECT";
        if !bound
            && (root.intent.account_id.is_none()
                || root.intent.request_key != format!("{}:inspect", root.intent.generation))
        {
            return Err(());
        }
        let mut original_q = None;
        let mut bound_action = None;
        let mut action_output = None;
        if bound {
            let single_mutation =
                ["CONFIGURE", "ENABLE", "RECOVERY"].contains(&root.intent.purpose.as_str());
            let base = format!("{JOURNAL}/operations/{}", root.intent.generation);
            if self.file(&format!("{base}/intent.json")).map_err(|_| ())? != rbytes {
                return Err(());
            }
            // Readback retains the actual original A and its arbitrary sequence.
            // Q precedes A in the producer's ordered input writes, so an
            // interrupted Q-only prefix resolves through the real dispatch.
            let sequence = if let Some(a) = a {
                wire::parse_action(a, &root)?.sequence
            } else if let Some(q) = qbytes {
                let raw = index.requests.get(&digest_bytes(q)).copied().ok_or(())?;
                let dispatch = wire::BoundDispatch::parse(raw)?;
                if dispatch.root.canonical.as_bytes() != rbytes
                    || dispatch.request.canonical.as_bytes() != q
                {
                    return Err(());
                }
                dispatch.action.sequence
            } else if single_mutation {
                0
            } else {
                // R or R/E can survive before Q/A identify which maintenance
                // action was read. Match only actual retained original actions;
                // no output/result is reachable before those ordered inputs.
                if has_output || result.is_some() || artifact.is_some() {
                    return Err(());
                }
                let actions = format!("{base}/actions");
                for name in self.children(&actions) {
                    self.check_time().map_err(|_| ())?;
                    let action_dir = format!("{actions}/{name}");
                    let binding = self
                        .file(&format!("{action_dir}/binding.json"))
                        .map_err(|_| ())?;
                    let action = wire::parse_action(binding, &root)?;
                    if name != format!("{:08}", action.sequence) {
                        return Err(());
                    }
                    let Ok(raw) = self.file(&format!("{action_dir}/output/dispatch.json")) else {
                        continue;
                    };
                    let dispatch = wire::BoundDispatch::parse(raw)?;
                    if dispatch.root.canonical.as_bytes() != rbytes
                        || dispatch.action.canonical.as_bytes() != binding
                    {
                        return Err(());
                    }
                    if let Some(e) = e {
                        if action.artifact_sha256.is_none() {
                            continue;
                        }
                        if self
                            .file(&format!("{action_dir}/evidence.json"))
                            .map_err(|_| ())?
                            != e
                        {
                            continue;
                        }
                        wire::parse_bound_artifact(e, &root, &action)?;
                    }
                    return Ok(());
                }
                return Err(());
            };
            let action_dir = format!("{base}/actions/{sequence:08}");
            let binding = self
                .file(&format!("{action_dir}/binding.json"))
                .map_err(|_| ())?;
            let action = wire::parse_action(binding, &root)?;
            if action.sequence != sequence || a.is_some_and(|a| a != binding) {
                return Err(());
            }
            match (action.artifact_sha256.as_deref(), e) {
                (Some(hash), Some(e)) => {
                    if self
                        .file(&format!("{action_dir}/evidence.json"))
                        .map_err(|_| ())?
                        != e
                        || hash != digest_bytes(e)
                    {
                        return Err(());
                    }
                    wire::parse_bound_artifact(e, &root, &action)?;
                }
                (Some(_), None) if a.is_some() || qbytes.is_some() || has_output => return Err(()),
                (None, Some(_)) => return Err(()),
                _ => (),
            }
            let out = format!("{action_dir}/output");
            if let Ok(bytes) = self.file(&format!("{out}/dispatch.json")) {
                let dispatch = wire::BoundDispatch::parse(bytes)?;
                if dispatch.root.canonical.as_bytes() != rbytes
                    || dispatch.action.canonical.as_bytes() != binding
                {
                    return Err(());
                }
                original_q = Some(dispatch.request);
            }
            if let Some(q) = qbytes {
                let parsed = wire::parse_exact_bound(q, &root, &action)?;
                if original_q
                    .as_ref()
                    .is_none_or(|o| o.canonical != parsed.canonical)
                {
                    return Err(());
                }
            }
            if (has_output || result.is_some()) && a.is_none() {
                return Err(());
            }
            // Only CONFIGURE/ENABLE/RECOVERY are new mutation submissions.
            // All other bound purposes here are descriptive readback inputs,
            // whose Q must have been written before A and output.
            if !single_mutation && (a.is_some() || has_output) && qbytes.is_none() {
                return Err(());
            }
            if result.as_ref().is_some_and(|r| r.verb != "readback")
                && (!single_mutation || action.sequence != 0)
            {
                return Err(());
            }
            if self.children(&output).next().is_some() {
                return Err(());
            }
            action_output = Some(out);
            bound_action = Some(action);
        } else {
            if a.is_some() {
                return Err(());
            }
            if let Some(e) = e {
                if qbytes.is_some()
                    || root.intent.artifact_sha256.as_deref() != Some(digest_bytes(e).as_str())
                {
                    return Err(());
                }
                wire::parse_process_binding(e, &root.intent)?;
            } else if root.intent.artifact_sha256.is_some() && (qbytes.is_some() || has_output) {
                return Err(());
            }
            if let Some(q) = qbytes {
                let q = wire::ExactRequest::parse(q)?;
                if q.value["kind"] != "RESET"
                    || root.intent.artifact_sha256.is_some()
                    || q.value["account_id"] != root.intent.value["account_id"]
                    || q.value["args"]["expected_epoch"] != root.intent.expected_epoch
                    || root.intent.account_id.is_none()
                {
                    return Err(());
                }
                original_q = Some(q);
            }
            if has_output && e.is_none() && qbytes.is_none() {
                return Err(());
            }
            if self.children(&output).take(2).count() > 1 {
                return Err(());
            }
            for name in self.children(&output) {
                self.check_time().map_err(|_| ())?;
                let bytes = self.file(&format!("{output}/{name}")).map_err(|_| ())?;
                if named_digest(name).as_deref() != Some(digest_bytes(bytes).as_str()) {
                    return Err(());
                }
                if e.is_some() {
                    let anchor = wire::ResetAnchor::parse(bytes)?;
                    self.lifecycle_observation(&root, e.ok_or(())?, &anchor, index)?;
                } else {
                    self.lifecycle_readback(bytes, original_q.as_ref().ok_or(())?, index)?;
                }
            }
        }
        if let Some(bytes) = artifact {
            let r = result.as_ref().ok_or(())?;
            if r.artifact_sha256 != digest_bytes(bytes) {
                return Err(());
            }
        }
        let Some(r) = result else {
            return Ok(());
        };
        if !r.generation.is_null() && r.generation != root.intent.generation.to_string() {
            return Err(());
        }
        if artifact.is_some() && r.artifact_sha256.is_null() {
            return Err(());
        }
        let selected = if let Some(hash) = r.artifact_sha256.as_str() {
            let original = self
                .file(&format!(
                    "{}/{}.json",
                    action_output.as_deref().unwrap_or(&output),
                    &hash[7..]
                ))
                .map_err(|_| ())?;
            if digest_bytes(original) != hash || artifact.is_some_and(|b| b != original) {
                return Err(());
            }
            Some(original)
        } else {
            None
        };
        if r.verb == "readback" {
            // Readback Q is an input, written before binding/output. Only a
            // mutation result may precede its later imported Q copy.
            if qbytes.is_none() {
                return Err(());
            }
            let q = original_q.as_ref().ok_or(())?;
            if r.generation != root.intent.generation.to_string() && !r.generation.is_null() {
                return Err(());
            }
            if let Some(bytes) = selected {
                let read = self.lifecycle_readback(bytes, q, index)?;
                if r.generation != root.intent.generation.to_string()
                    || r.event_id != read["event_id"]
                    || r.control_epoch != read["current_epoch"]
                    || match read["status"].as_str() {
                        Some("RESET") => !["OK", "COMMIT_UNKNOWN"].contains(&r.outcome.as_str()),
                        Some("COMMITTED") => {
                            !["REPLAY", "COMMIT_UNKNOWN"].contains(&r.outcome.as_str())
                        }
                        _ => r.outcome != "COMMIT_UNKNOWN",
                    }
                {
                    return Err(());
                }
            } else if r.success() || !r.event_id.is_null() || !r.control_epoch.is_null() {
                return Err(());
            }
        } else if r.verb == "observe-reset" {
            if bound || e.is_none() || qbytes.is_some() {
                return Err(());
            }
            if let Some(bytes) = selected {
                let anchor = wire::ResetAnchor::parse(bytes)?;
                self.lifecycle_observation(&root, e.ok_or(())?, &anchor, index)?;
                if r.outcome != "OK"
                    || r.event_id != anchor.selection.reset.event_id.to_string()
                    || r.generation != root.intent.generation.to_string()
                    || r.control_epoch != root.intent.expected_epoch
                {
                    return Err(());
                }
            } else if r.success() {
                return Err(());
            }
        } else {
            let action = bound_action.as_ref().ok_or(())?;
            if r.verb != action.verb.as_str() {
                return Err(());
            }
            if r.success()
                || selected.is_some()
                || !r.event_id.is_null()
                || !r.control_epoch.is_null()
            {
                if !r.success() && r.outcome != "COMMIT_UNKNOWN" {
                    return Err(());
                }
                let q = original_q.as_ref().ok_or(())?;
                let rows: Vec<_> = index
                    .rows
                    .iter()
                    .filter(|(_, row)| row["request_sha256"] == q.sha256)
                    .collect();
                if rows.len() != 1 {
                    return Err(());
                }
                let (table, row) = rows[0];
                self.check_receipt(table, row, index).map_err(|_| ())?;
                let sql = wire::parse_canonical_limit(
                    row["result_canonical"].as_str().ok_or(())?.as_bytes(),
                    wire::MAX_EXACT_BYTES,
                )?;
                if selected.is_none() && r.success() {
                    return Err(());
                }
                if r.event_id != row["id"] {
                    return Err(());
                }
                match r.verb.as_str() {
                    "configure" => {
                        if !r.generation.is_null()
                            || r.control_epoch != 0
                            || selected.is_some_and(|bytes| Some(bytes) != e)
                        {
                            return Err(());
                        }
                    }
                    "enable" => {
                        if r.generation != root.intent.generation.to_string()
                            || r.control_epoch != sql["control_epoch"]
                            || selected.is_some_and(|bytes| {
                                Some(bytes) != row["result_canonical"].as_str().map(str::as_bytes)
                            })
                        {
                            return Err(());
                        }
                    }
                    "recover" => {
                        if r.generation != root.intent.generation.to_string() {
                            return Err(());
                        }
                        if let Some(bytes) = selected {
                            if r.outcome == "COMMIT_UNKNOWN"
                                && Some(bytes)
                                    == row["result_canonical"].as_str().map(str::as_bytes)
                            {
                                if r.control_epoch != sql["control_epoch"] {
                                    return Err(());
                                }
                            } else {
                                let read = self.lifecycle_readback(bytes, q, index)?;
                                if read["status"] != "COMMITTED"
                                    || r.control_epoch != read["current_epoch"]
                                    || (r.outcome == "OK"
                                        && (read["current_epoch"] != action.expected_epoch
                                            || read["current_state"] != "ENABLED"))
                                {
                                    return Err(());
                                }
                            }
                        } else if r
                            .control_epoch
                            .as_i64()
                            .is_none_or(|n| n < sql["control_epoch"].as_i64().unwrap_or(i64::MAX))
                        {
                            return Err(());
                        }
                    }
                    _ => return Err(()),
                }
            }
        }
        Ok(())
    }

    fn lifecycle_observation(
        &self,
        root: &wire::RootIntent,
        evidence: &[u8],
        anchor: &wire::ResetAnchor,
        index: &JournalIndex<'_>,
    ) -> Result<(), ()> {
        let process = wire::parse_process_binding(evidence, &root.intent)?;
        let s = &anchor.selection;
        if s.installation_id != root.intent.installation_id
            || Some(s.reset.account_id) != root.intent.account_id
            || s.reset.control_epoch > root.intent.expected_epoch
            || s.release_manifest_sha256 != root.intent.release_manifest_sha256
            || process.value["daily_cohort_sha256"] != s.cohort_sha256
        {
            return Err(());
        }
        self.lifecycle_reset(&s.reset, index)?;
        let mut found = 0;
        for name in self.children(&format!("{LIFECYCLE}/selections")) {
            self.check_time().map_err(|_| ())?;
            let raw = self
                .file(&format!("{LIFECYCLE}/selections/{name}"))
                .map_err(|_| ())?;
            if selection_equal(&wire::ResetSelection::parse(raw)?, s) {
                found += 1;
            }
        }
        if found != 1 {
            return Err(());
        }
        Ok(())
    }

    pub(super) fn lifecycle_enable(
        &self,
        root: &wire::RootIntent,
        q: &wire::ExactRequest,
        selections: &[wire::ResetSelection],
    ) -> Result<(), ()> {
        let evidence = wire::parse_canonical_limit(
            q.value["args"]["evidence_canonical"]
                .as_str()
                .ok_or(())?
                .as_bytes(),
            wire::MAX_EXACT_BYTES,
        )?;
        let sessions = evidence["session_evidence"].as_array().ok_or(())?;
        let mut matching = 0;
        for selection in selections {
            self.check_time().map_err(|_| ())?;
            if selection.installation_id != root.intent.installation_id
                || Some(selection.reset.account_id) != root.intent.account_id
                || selection.release_manifest_sha256 != root.intent.release_manifest_sha256
                || selection.reset.control_epoch != root.intent.expected_epoch
            {
                continue;
            }
            let mut all = true;
            for session in sessions {
                let mut arms = Vec::new();
                let mut slot = None;
                for ((table, _), raw) in &self.rows {
                    self.check_time().map_err(|_| ())?;
                    if table == "owner_beta_paper_source_runs" {
                        let row: Value = serde_json::from_slice(raw).map_err(|_| ())?;
                        if row["run_id"] == session["run_id"] {
                            if slot.is_some() {
                                return Err(());
                            }
                            slot = Some(row["slot_id"].clone());
                        }
                    }
                }
                let mut slot_row = None;
                let mut admission_row = None;
                for ((table, _), raw) in &self.rows {
                    self.check_time().map_err(|_| ())?;
                    if [
                        "owner_beta_paper_source_slots",
                        "owner_beta_paper_source_admissions",
                    ]
                    .contains(&table.as_str())
                    {
                        let row: Value = serde_json::from_slice(raw).map_err(|_| ())?;
                        if slot.as_ref() == Some(&row["slot_id"]) {
                            let target = if table == "owner_beta_paper_source_slots" {
                                &mut slot_row
                            } else {
                                &mut admission_row
                            };
                            if target.is_some() {
                                return Err(());
                            }
                            *target = Some(row);
                        }
                    }
                }
                let slot_row = slot_row.ok_or(())?;
                let admission_row = admission_row.ok_or(())?;
                if session["session_date"] != slot_row["session_date"]
                    || session["root_event_sha256"] != slot_row["root_event_sha256"]
                    || session["source_event_sha256"] != admission_row["source_event_sha256"]
                    || session["admission_sha256"] != admission_row["admission_sha256"]
                    || slot_row["cohort"] != evidence["current_cohort"]
                {
                    return Err(());
                }
                for (path, member) in &self.members {
                    self.check_time().map_err(|_| ())?;
                    if !path.starts_with("paper_evidence/source-") {
                        continue;
                    }
                    let Some(raw) = &member.bytes else {
                        continue;
                    };
                    let event = market_data::paper_source_evidence::decode_canonical(
                        raw,
                        market_data::paper_source_evidence::SOURCE_LIMIT,
                    )
                    .map_err(|_| ())?;
                    if slot.as_ref() == Some(&event["slot_id"])
                        && ["ARM_INTENT", "ARMED"].iter().any(|k| event["kind"] == *k)
                    {
                        arms.push(event);
                    }
                }
                if arms.len() != 2
                    || arms[0]["kind"] == arms[1]["kind"]
                    || arms.iter().any(|e| {
                        e["cohort"]["cohort_sha256"] != selection.cohort_sha256
                            || e["cohort"] != evidence["current_cohort"]
                            || e["session"] != session["session_date"]
                    })
                {
                    all = false;
                    break;
                }
                for arm in arms {
                    let mut found = 0;
                    for name in self.children(&format!("{LIFECYCLE}/anchors")) {
                        let bytes = self
                            .file(&format!("{LIFECYCLE}/anchors/{name}"))
                            .map_err(|_| ())?;
                        let anchor = wire::ResetAnchor::parse(bytes)?;
                        let clock = crate::owner_beta::paper::evidence::ClockTriple::from_value(
                            &arm["observed"],
                        )
                        .map_err(|_| ())?;
                        if selection_equal(selection, &anchor.selection)
                            && arm["arm"]["boot_id"] == anchor.boot_id.to_string()
                            && clock.strictly_after(&anchor.observed)
                        {
                            found += 1;
                        }
                    }
                    if found != 1 {
                        all = false;
                        break;
                    }
                }
            }
            if all {
                matching += 1;
            }
        }
        if matching == 1 { Ok(()) } else { Err(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_stdout_is_closed_producer_order_and_explicit_null_not_canonical_sort() {
        let raw = br#"{"schema":"owner-beta-paper-ops-query-result-v1","verb":"readback","outcome":"COMMIT_UNKNOWN","event_id":null,"generation":null,"control_epoch":null,"artifact_sha256":null,"error_code":"COMMIT_UNKNOWN"}"#;
        assert!(NativeResult::parse(raw).is_ok());
        let mut lf = raw.to_vec();
        lf.push(b'\n');
        assert!(NativeResult::parse(&lf).is_ok());
        for mutant in [
            String::from_utf8(raw.to_vec())
                .unwrap()
                .replace("\"event_id\":null,", ""),
            String::from_utf8(raw.to_vec())
                .unwrap()
                .replace("\"event_id\":null,", "\"event_id\":null,\"event_id\":null,"),
            String::from_utf8(raw.to_vec())
                .unwrap()
                .replace("\"event_id\":null,", "\"unknown\":null,\"event_id\":null,"),
            String::from_utf8(raw.to_vec())
                .unwrap()
                .replace("\"control_epoch\":null", "\"control_epoch\":true"),
        ] {
            assert!(NativeResult::parse(mutant.as_bytes()).is_err());
        }
        let sorted: Value = serde_json::from_slice(raw).unwrap();
        assert!(NativeResult::parse(&serde_json::to_vec(&sorted).unwrap()).is_err());
    }
}
