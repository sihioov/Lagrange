use super::*;
use serde_json::json;
use crate::owner_beta::paper::source_evidence::{canonical, digest as hash};

fn fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
    )))
    .unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    canonical(value).unwrap()
}
fn original<'a>(v: &'a Value, name: &str) -> Result<&'a str, ()> {
    text(v["objects"].get(name).ok_or(())?, "canonical")
}
fn object(v: &Value, name: &str) -> Result<Value, ()> {
    parse_canonical_limit(original(v, name)?.as_bytes(), MAX_DISPATCH_BYTES)
}
fn name<'a>(v: &'a Value, field: &str) -> Result<&'a str, ()> {
    text(v, field)
}
fn object_hash(v: &Value, name: &str) -> Result<String, ()> {
    Ok(hash(original(v, name)?.as_bytes()))
}

// These extra fixture checks test the adopted caller/history cross-links. They
// do not construct a live native chain or a stopped/recovery authority value.
fn caller(v: &Value, trace: &Value, root: &RootIntent) -> Result<(), ()> {
    let c = object(v, name(trace, "caller")?)?;
    exact_keys(
        &c,
        &[
            "schema",
            "installation_id",
            "account_id",
            "caller_key",
            "generation",
            "root_intent_sha256",
            "command_canonical",
        ],
    )?;
    if c["schema"] != "owner-beta-paper-caller-binding-v1" {
        return Err(());
    }
    request_key(text(&c, "caller_key")?)?;
    for key in ["installation_id", "account_id", "generation"] {
        if c[key] != root.intent.value[key] {
            return Err(());
        }
    }
    let mut projection = root.intent.value.clone();
    projection.as_object_mut().ok_or(())?.remove("generation");
    projection.as_object_mut().ok_or(())?.remove("request_key");
    if text(&c, "root_intent_sha256")? != root.sha256
        || text(&c, "command_canonical")?.as_bytes() != bytes(&projection)
        || (root.intent.purpose == "CONFIGURE" && c["caller_key"] != root.intent.request_key)
    {
        return Err(());
    }
    Ok(())
}

fn stage(
    v: &Value,
    name: &str,
    root: &RootIntent,
    sequence: i64,
    previous: Option<&str>,
) -> Result<Value, ()> {
    let st = object(v, name)?;
    exact_keys(
        &st,
        &[
            "schema",
            "installation_id",
            "generation",
            "account_id",
            "intent_sha256",
            "sequence",
            "previous_sha256",
            "stage",
            "control_epoch",
            "event_id",
            "artifact_sha256",
            "boot_id",
            "observed",
            "error_code",
        ],
    )?;
    if st["schema"] != "owner-beta-paper-operation-stage-v1"
        || st["sequence"] != sequence
        || st["previous_sha256"].as_str() != previous
        || st["intent_sha256"] != root.sha256
    {
        return Err(());
    }
    for key in ["installation_id", "generation", "account_id"] {
        if st[key] != root.intent.value[key] {
            return Err(());
        }
    }
    let kind = text(&st, "stage")?;
    let epoch = match kind {
        "INTENT" => root.intent.expected_epoch,
        "OFF_COMMITTED" | "RESTORED_COMMITTED" | "SNAPSHOT" => {
            root.fence_epoch()?.checked_add(1).ok_or(())?
        }
        "FENCED" | "STOPPED" | "RECONCILED" | "RESTORE_VERIFIED" => root.fence_epoch()?,
        _ => return Err(()),
    };
    if st["control_epoch"] != epoch || st["error_code"] != "NONE" {
        return Err(());
    }
    optional_uuid(&st, "event_id")?;
    parse_uuid(text(&st, "boot_id")?)?;
    optional_digest(&st, "artifact_sha256")?;
    validate_clock(&st["observed"])?;
    Ok(st)
}

fn summary_by_hash(v: &Value, digest: &str) -> Result<Value, ()> {
    let objs = v["objects"].as_object().ok_or(())?;
    let (_, item) = objs
        .iter()
        .find(|(_, item)| item["sha256"] == digest)
        .ok_or(())?;
    let value = parse_canonical(text(item, "canonical")?.as_bytes())?;
    validate_summary(&value)?;
    Ok(value)
}

fn readback_for(request: &ExactRequest, result: &str) -> Result<ExactReadback, ()> {
    let r = parse_canonical_limit(result.as_bytes(), MAX_PAGE_BYTES)?;
    let id = match request.value["kind"].as_str().ok_or(())? {
        "BEGIN" => &r["data"]["operation_event_id"],
        "RECONCILE" => &r["data"]["observation_id"],
        _ => &r["data"]["event_id"],
    };
    ExactReadback::parse(
        serde_json::json!({
            "schema":"owner-beta-paper-exact-readback-v1", "kind":request.value["kind"],
            "request_sha256":request.sha256, "status":"COMMITTED", "account_id":r["account_id"],
            "current_epoch":r["control_epoch"], "current_state":"OFF", "event_id":id,
            "event_epoch":r["control_epoch"], "result_canonical":result, "result_sha256":hash(result.as_bytes()), "reset":null,
        }),
        request,
    )
}

fn source_restore(v: &Value, trace: &Value, root: &RootIntent, receipt: &Value) -> Result<(), ()> {
    let source = name(trace, "source")?;
    let r = object(v, &format!("{source}.root"))?;
    let off = object(v, &format!("{source}.receipt"))?;
    let archive = object(v, &format!("{source}.archive"))?;
    let result = object(v, &format!("{source}.result"))?;
    let source_fence = i64_value(&off, "resulting_epoch")?;
    if receipt["source_installation_id"] != r["installation_id"]
        || receipt["source_generation"] != r["generation"]
        || receipt["source_epoch"] != source_fence
        || result["control_epoch"].as_i64() != source_fence.checked_add(1)
        || archive["control_epoch"] != result["control_epoch"]
        || archive["drain_receipt_sha256"] != object_hash(v, &format!("{source}.receipt"))?
        || archive["installation_id"] != receipt["source_installation_id"]
        || archive["generation"] != receipt["source_generation"]
        || archive["account_id"] != root.intent.value["account_id"]
        || root.intent.artifact_sha256.as_deref()
            != Some(object_hash(v, &format!("{source}.archive"))?.as_str())
        || archive["audit_sha256"] != receipt["source_audit_sha256"]
        || i64_value(&archive, "control_epoch")? > root.intent.local_epoch_floor
    {
        return Err(());
    }
    Ok(())
}

fn validate_fixture(v: &Value) -> Result<(), ()> {
    if v["authority"] != "SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN" {
        return Err(());
    }
    for (_, item) in v["objects"].as_object().ok_or(())? {
        let canonical = text(item, "canonical")?;
        parse_canonical_limit(canonical.as_bytes(), MAX_DISPATCH_BYTES)?;
        if hash(canonical.as_bytes()) != text(item, "sha256")? {
            return Err(());
        }
    }
    for trace in v["traces"].as_array().ok_or(())? {
        let root = parse_root_intent(original(v, name(trace, "root")?)?.as_bytes())?;
        caller(v, trace, &root)?;
        let mut stages: Vec<(String, Value)> = Vec::new();
        for (n, s) in trace["stages"].as_array().ok_or(())?.iter().enumerate() {
            let sn = s.as_str().ok_or(())?;
            let st = stage(
                v,
                sn,
                &root,
                i64::try_from(n).map_err(|_| ())?,
                stages.last().map(|(h, _)| h.as_str()),
            )?;
            stages.push((object_hash(v, sn)?, st));
        }
        if root.maintenance() == stages.is_empty() {
            return Err(());
        }
        let mut previous: Option<String> = None;
        let mut last_stage = 0;
        let actions = trace["actions"].as_array().ok_or(())?;
        if !root.maintenance() && actions.len() != 1 {
            return Err(());
        }
        for (n, entry) in actions.iter().enumerate() {
            let action = parse_action(original(v, name(entry, "binding")?)?.as_bytes(), &root)?;
            if action.sequence != i64::try_from(n).map_err(|_| ())?
                || action.previous_binding_sha256 != previous
            {
                return Err(());
            }
            previous = Some(action.sha256.clone());
            let dispatch = BoundDispatch::parse(original(v, name(entry, "dispatch")?)?.as_bytes())?;
            let sql = original(v, name(entry, "sql")?)?;
            if dispatch.root.canonical != root.canonical
                || dispatch.action.canonical != action.canonical
                || dispatch.request.canonical != sql
            {
                return Err(());
            }
            let artifact = match &entry["evidence"] {
                Value::Null => None,
                Value::String(e) => Some(parse_bound_artifact(
                    original(v, e)?.as_bytes(),
                    &root,
                    &action,
                )?),
                _ => return Err(()),
            };
            correlate_bound(&root, &action, artifact.as_ref(), &dispatch.request)?;
            if let Some(digest) = &action.stage_sha256 {
                let (index, (_, st)) = stages
                    .iter()
                    .enumerate()
                    .find(|(_, (h, _))| h == digest)
                    .ok_or(())?;
                if index < last_stage {
                    return Err(());
                }
                last_stage = index;
                let allowed: &[&str] = match action.verb {
                    Verb::Begin => &["INTENT"],
                    Verb::Reconcile => &["STOPPED", "RECONCILED"],
                    Verb::Off => &["RECONCILED"],
                    Verb::RestoreOff => &["RESTORE_VERIFIED"],
                    _ => return Err(()),
                };
                if !allowed.contains(&text(st, "stage")?) {
                    return Err(());
                }
                if matches!(action.verb, Verb::Off | Verb::RestoreOff) {
                    let e = &artifact.as_ref().ok_or(())?.value;
                    let latest = stages[..=index]
                        .iter()
                        .rev()
                        .find(|(_, st)| st["stage"] == "RECONCILED")
                        .ok_or(())?;
                    if latest.1["artifact_sha256"] != e["reconciliation_sha256"] {
                        return Err(());
                    }
                    let summary = summary_by_hash(v, text(e, "reconciliation_sha256")?)?;
                    if summary["inventory_sha256"] != e["inventory_sha256"]
                        || summary["observation_id"] != e["observation_id"]
                        || summary["control_epoch"] != root.fence_epoch()?
                        || summary["unresolved_commits"] != 0
                        || summary["live_claims"] != 0
                    {
                        return Err(());
                    }
                    if action.verb == Verb::RestoreOff {
                        source_restore(v, trace, &root, e)?;
                    }
                }
            }
            if n == 0 && action.verb == Verb::Begin {
                readback_for(
                    &dispatch.request,
                    original(v, &format!("{}.begin-result", name(trace, "name")?))?,
                )?;
            }
            if n + 1 == actions.len() {
                readback_for(
                    &dispatch.request,
                    original(v, &format!("{}.result", name(trace, "name")?))?,
                )?;
            }
        }
    }
    Ok(())
}

#[test]
fn shared_six_traces_preserve_all_96_original_preimages() {
    let v = fixture();
    assert_eq!(v["objects"].as_object().unwrap().len(), 96);
    assert_eq!(v["traces"].as_array().unwrap().len(), 6);
    assert_eq!(validate_fixture(&v), Ok(()));
    for trace in v["traces"].as_array().unwrap() {
        for a in trace["actions"].as_array().unwrap() {
            let d = BoundDispatch::parse(
                original(&v, name(a, "dispatch").unwrap())
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
            let rebuilt =
                BoundDispatch::build(&d.root, &d.action, d.artifact.as_ref(), &d.request).unwrap();
            assert_eq!(rebuilt.canonical, d.canonical);
            assert_eq!(rebuilt.sha256, d.sha256);
        }
    }
}

fn replace_object(v: &mut Value, n: &str, data: &Value) {
    let wire = String::from_utf8(bytes(data)).unwrap();
    v["objects"][n]["sha256"] = hash(wire.as_bytes()).into();
    v["objects"][n]["canonical"] = wire.into();
}

#[test]
fn shared_44_substitutions_reject_even_with_rehashed_dispatch_wrappers() {
    let base = fixture();
    assert_eq!(base["negative_cases"].as_array().unwrap().len(), 44);
    for negative in base["negative_cases"].as_array().unwrap() {
        let mut v = base.clone();
        let n = name(negative, "object").unwrap();
        match name(negative, "mode").unwrap() {
            "remove-object" => {
                v["objects"].as_object_mut().unwrap().remove(n);
            }
            "duplicate-action" => {
                let a = v["traces"][0]["actions"][1].clone();
                v["traces"][0]["actions"]
                    .as_array_mut()
                    .unwrap()
                    .insert(2, a);
            }
            "replace" => {
                let mut data = object(&v, n).unwrap();
                data[name(negative, "field").unwrap()] = negative["replacement"].clone();
                replace_object(&mut v, n, &data);
                // As in the adopted vectors, refresh the wrappers too: tests
                // must reject changed semantics, not merely stale index hashes.
                for trace in base["traces"].as_array().unwrap() {
                    for entry in trace["actions"].as_array().unwrap() {
                        let dn = name(entry, "dispatch").unwrap();
                        let mut d = object(&v, dn).unwrap();
                        for (field, reference) in [
                            ("root_intent", &trace["root"]),
                            ("action", &entry["binding"]),
                            ("artifact", &entry["evidence"]),
                            ("sql_request", &entry["sql"]),
                        ] {
                            if let Some(reference) = reference.as_str() {
                                d[format!("{field}_canonical")] =
                                    v["objects"][reference]["canonical"].clone();
                                d[format!("{field}_sha256")] =
                                    v["objects"][reference]["sha256"].clone();
                            }
                        }
                        replace_object(&mut v, dn, &d);
                    }
                }
            }
            _ => panic!("unknown adopted negative mode"),
        }
        assert!(validate_fixture(&v).is_err(), "{}", negative["name"]);
    }
}

#[test]
fn bound_restore_floor_nine_fence_ten_off_eleven_keeps_source_fence_one_archive_two() {
    let v = fixture();
    let d =
        BoundDispatch::parse(original(&v, "restore.action2.dispatch").unwrap().as_bytes()).unwrap();
    assert_eq!(d.root.intent.expected_epoch, 2);
    assert_eq!(d.root.intent.local_epoch_floor, 9);
    assert_eq!(d.root.fence_epoch(), Ok(10));
    assert_eq!(d.action.expected_epoch, 10);
    let e = &d.artifact.as_ref().unwrap().value;
    assert_eq!(e["expected_epoch"], 2);
    assert_eq!(e["resulting_epoch"], 10);
    assert_eq!(e["source_epoch"], 1);
    assert_eq!(object(&v, "backup.archive").unwrap()["control_epoch"], 2);
    let rb = readback_for(&d.request, original(&v, "restore.result").unwrap()).unwrap();
    assert_eq!(rb.value["event_epoch"], 11);
    assert!(ExactRequest::parse(d.request.canonical.as_bytes()).is_err());
    let begin =
        BoundDispatch::parse(original(&v, "restore.action0.dispatch").unwrap().as_bytes()).unwrap();
    assert!(begin.root.intent.artifact_sha256.is_some());
    assert!(begin.artifact.is_none());
    assert!(begin.action.artifact_sha256.is_none());
    assert_eq!(
        begin.request.value["args"]["intent_canonical"],
        begin.root.canonical
    );
}

#[test]
fn bound_configure_keeps_null_account_and_null_process_sql_argument() {
    let v = fixture();
    let d = BoundDispatch::parse(
        original(&v, "configure.action0.dispatch")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    assert!(d.root.intent.account_id.is_none());
    assert!(d.request.value["account_id"].is_null());
    assert!(d.request.value["args"]["process_binding"].is_null());
    // The production configure call retains its literal SQL NULL argument;
    // actual DB conversion is a hosted PostgreSQL assertion, not this test.
    let rb = readback_for(&d.request, original(&v, "configure.result").unwrap()).unwrap();
    assert!(!rb.value["account_id"].is_null());
    assert!(d.request.value["account_id"].is_null());
}

#[test]
fn bound_historical_readback_keeps_expired_fractional_enable_and_later_current_epoch() {
    let v = fixture();
    let d =
        BoundDispatch::parse(original(&v, "enable.action0.dispatch").unwrap().as_bytes()).unwrap();
    let before = d.request.canonical.clone();
    for _scenario in v["readback"]["scenarios"].as_array().unwrap() {
        let rb =
            ExactReadback::parse(object(&v, "readback.committed").unwrap(), &d.request).unwrap();
        rb.correlate_original(&d.root.intent).unwrap();
        assert_eq!(rb.value["event_epoch"], 3);
        assert_eq!(rb.value["current_epoch"], 5);
        assert_eq!(rb.value["current_state"], "OFF");
        assert_eq!(d.request.canonical, before);
        assert!(d.request.canonical.contains(".123456Z"));
    }
    let absent = ExactReadback::parse(object(&v, "readback.absent").unwrap(), &d.request).unwrap();
    assert_eq!(absent.value["status"], "ABSENT");
    assert!(absent.value["result_canonical"].is_null());
}

#[test]
fn bound_root_action_sql_and_caller_domains_cannot_be_swapped() {
    let v = fixture();
    let root = parse_root_intent(original(&v, "stop.root").unwrap().as_bytes()).unwrap();
    let a = parse_action(original(&v, "stop.action1").unwrap().as_bytes(), &root).unwrap();
    for other in ["backup.action1", "stop.action2", "stop.action3"] {
        let other_root = if other.starts_with("backup") {
            parse_root_intent(original(&v, "backup.root").unwrap().as_bytes()).unwrap()
        } else {
            parse_root_intent(original(&v, "stop.root").unwrap().as_bytes()).unwrap()
        };
        let other_action =
            parse_action(original(&v, other).unwrap().as_bytes(), &other_root).unwrap();
        assert!(
            parse_exact_bound(
                original(&v, "stop.action1.sql").unwrap().as_bytes(),
                &root,
                &other_action
            )
            .is_err()
        );
    }
    assert!(
        parse_exact_bound(
            original(&v, "backup.action1.sql").unwrap().as_bytes(),
            &root,
            &a
        )
        .is_err()
    );
    let mut replaced = object(&v, "stop.root").unwrap();
    replaced["request_key"] = object(&v, "stop.caller").unwrap()["caller_key"].clone();
    assert!(parse_root_intent(&bytes(&replaced)).is_err());
    assert_eq!(
        root.intent.request_key,
        format!("{}:begin", root.intent.generation)
    );
}

#[test]
fn bound_bounds_and_canonical_rules_reject_unknown_fields_overflow_and_bad_ordinals() {
    let v = fixture();
    let root_bytes = original(&v, "stop.root").unwrap().as_bytes();
    let root = parse_root_intent(root_bytes).unwrap();
    for raw in [
        b"{\"a\":1,\"a\":1}".as_slice(),
        b"{\"a\":1.0}",
        b"{\"a\":-0}",
        b"{\"a\":1e0}",
    ] {
        assert!(parse_root_intent(raw).is_err());
    }
    let mut lf = root_bytes.to_vec();
    lf.push(b'\n');
    assert!(parse_root_intent(&lf).is_err());
    for key in ["expected_epoch", "local_epoch_floor"] {
        let mut r = root.intent.value.clone();
        r[key] = i64::MAX.into();
        assert!(parse_root_intent(&bytes(&r)).is_err());
    }
    for suffix in ["0", "01", "-1", "1e0", "1000000000000000000"] {
        let mut a = object(&v, "stop.action1").unwrap();
        a["request_key"] = format!("{}:reconcile:{suffix}", root.intent.generation).into();
        assert!(parse_action(&bytes(&a), &root).is_err());
    }
    let mut unknown = object(&v, "stop.action1").unwrap();
    unknown["ready"] = true.into();
    assert!(parse_action(&bytes(&unknown), &root).is_err());
    assert!(parse_action(&vec![b'x'; MAX_WIRE_BYTES + 1], &root).is_err());
}

#[test]
fn bound_dispatch_and_floor_ahead_exact_result_reject_partial_pairs_and_epoch_swaps() {
    let v = fixture();
    let d =
        BoundDispatch::parse(original(&v, "restore.action2.dispatch").unwrap().as_bytes()).unwrap();
    let rb = readback_for(&d.request, original(&v, "restore.result").unwrap()).unwrap();
    for epoch in [2, 3, 9, 10, 12] {
        let mut result = object(&v, "restore.result").unwrap();
        result["control_epoch"] = epoch.into();
        let mut changed = rb.value.clone();
        changed["event_epoch"] = epoch.into();
        changed["current_epoch"] = 20.into();
        let text = String::from_utf8(bytes(&result)).unwrap();
        changed["result_sha256"] = hash(text.as_bytes()).into();
        changed["result_canonical"] = text.into();
        assert!(
            ExactReadback::parse(changed, &d.request).is_err(),
            "epoch {epoch}"
        );
    }
    let complete = object(&v, "restore.action2.dispatch").unwrap();
    for key in [
        "root_intent_canonical",
        "action_canonical",
        "artifact_canonical",
        "artifact_sha256",
        "sql_request_canonical",
    ] {
        let mut partial = complete.clone();
        partial[key] = Value::Null;
        assert!(BoundDispatch::parse(&bytes(&partial)).is_err(), "{key}");
    }
    let mut extra = complete;
    extra["verified"] = true.into();
    assert!(BoundDispatch::parse(&bytes(&extra)).is_err());
    let mut lf = original(&v, "restore.action2.dispatch")
        .unwrap()
        .as_bytes()
        .to_vec();
    lf.push(b'\n');
    assert!(BoundDispatch::parse(&lf).is_err());
    assert!(BoundDispatch::parse(&vec![b'x'; MAX_DISPATCH_BYTES + 1]).is_err());
}

#[test]
fn legacy_exact_preimages_and_v1_custody_remain_separate_from_bound_v2() {
    let v = fixture();
    for (r, q, verb) in [
        ("stop.root", "stop.action0.sql", Verb::Begin),
        ("configure.root", "configure.action0.sql", Verb::Configure),
        ("enable.root", "enable.action0.sql", Verb::Enable),
        ("recover.root", "recover.action0.sql", Verb::Recover),
    ] {
        let root = parse_intent(original(&v, r).unwrap().as_bytes(), verb).unwrap();
        let request = ExactRequest::parse(original(&v, q).unwrap().as_bytes()).unwrap();
        assert_eq!(
            request.canonical.as_bytes(),
            original(&v, q).unwrap().as_bytes()
        );
        assert_eq!(root.value, object(&v, r).unwrap());
    }
    let d = BoundDispatch::parse(
        original(&v, "configure.action0.dispatch")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let old = serde_json::json!({"schema":"owner-beta-paper-native-dispatch-v1", "intent_canonical":d.root.canonical,
        "intent_sha256":d.root.sha256, "artifact_canonical":original(&v, "configure.artifact").unwrap(),
        "artifact_sha256":d.action.artifact_sha256, "sql_request_canonical":d.request.canonical, "sql_request_sha256":d.request.sha256});
    assert!(BoundDispatch::parse(&bytes(&old)).is_err());
    // Legacy parser semantics remain unchanged, including dormant BACKUP.
    assert!(parse_intent(original(&v, "backup.root").unwrap().as_bytes(), Verb::Begin).is_err());
    assert!(parse_root_intent(original(&v, "backup.root").unwrap().as_bytes()).is_ok());
}


#[test]
fn reconcile_and_off_keep_e_f_z_and_each_original_domain_separate() {
    let vectors = fixture();
    for (prefix, off_sequence) in [("stop", 3), ("backup", 2)] {
        let root = parse_root_intent(original(&vectors, &format!("{prefix}.root")).unwrap().as_bytes()).unwrap();
        let action = parse_action(original(&vectors, &format!("{prefix}.action{off_sequence}")).unwrap().as_bytes(), &root).unwrap();
        let evidence = original(&vectors, &format!("{prefix}.receipt")).unwrap();
        let artifact = parse_bound_artifact(evidence.as_bytes(), &root, &action).unwrap();
        let q = parse_exact_bound(original(&vectors, &format!("{prefix}.action{off_sequence}.sql")).unwrap().as_bytes(), &root, &action).unwrap();
        correlate_bound(&root, &action, Some(&artifact), &q).unwrap();
        assert_eq!(q.value["args"]["expected_epoch"], root.fence_epoch().unwrap());
        assert_eq!(artifact.value["expected_epoch"], root.intent.expected_epoch);
        assert_eq!(artifact.value["resulting_epoch"], root.fence_epoch().unwrap());
        assert_eq!(artifact.value["intent_sha256"], root.sha256);
        assert_eq!(q.value["args"]["evidence_canonical"], evidence);
        for field in ["expected_epoch", "resulting_epoch"] {
            let mut changed = artifact.value.clone();
            changed[field] = json!(root.fence_epoch().unwrap() + 1);
            let mut binding: Value = serde_json::from_str(&action.canonical).unwrap();
            binding["artifact_sha256"] = json!(hash(&bytes(&changed)));
            let binding = parse_action(&bytes(&binding), &root).unwrap();
            assert!(parse_bound_artifact(&bytes(&changed), &root, &binding).is_err());
        }
        let mut changed = artifact.value.clone();
        changed["intent_sha256"] = json!(action.sha256);
        let mut binding: Value = serde_json::from_str(&action.canonical).unwrap();
        binding["artifact_sha256"] = json!(hash(&bytes(&changed)));
        let binding = parse_action(&bytes(&binding), &root).unwrap();
        assert!(parse_bound_artifact(&bytes(&changed), &root, &binding).is_err());
    }
}

#[test]
fn maintenance_ordinal_is_minimal_bounded_and_fence_overflow_rejected() {
    let vectors = fixture();
    let root = parse_root_intent(original(&vectors, "stop.root").unwrap().as_bytes()).unwrap();
    let good = object(&vectors, "stop.action1").unwrap();
    for suffix in ["0", "01", "+1", "1.0", "1000000000000000000"] {
        let mut bad = good.clone();
        bad["request_key"] = json!(format!("{}:reconcile:{suffix}", root.intent.generation));
        assert!(parse_action(&bytes(&bad), &root).is_err());
    }
    let mut too_high = root.intent.value.clone();
    too_high["expected_epoch"] = json!(i64::MAX);
    too_high["local_epoch_floor"] = json!(i64::MAX);
    assert!(parse_root_intent(&bytes(&too_high)).is_err());
    let mut last = root.intent.value.clone();
    last["expected_epoch"] = json!(i64::MAX - 1);
    last["local_epoch_floor"] = json!(i64::MAX - 1);
    assert!(parse_root_intent(&bytes(&last)).is_err());
    last["expected_epoch"] = json!(i64::MAX - 2);
    last["local_epoch_floor"] = json!(i64::MAX - 2);
    let last = parse_root_intent(&bytes(&last)).unwrap();
    assert_eq!(last.fence_epoch().unwrap(), i64::MAX - 1);
    assert_eq!(last.fence_epoch().unwrap().checked_add(1), Some(i64::MAX));
}
