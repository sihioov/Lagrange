use super::*;

fn fixture_dispatch(name: &str) -> BoundDispatch {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
    )))
    .unwrap();
    BoundDispatch::parse(
        fixture["objects"][name]["canonical"]
            .as_str()
            .unwrap()
            .as_bytes(),
    )
    .unwrap()
}

#[test]
fn existing_dispatch_uncertainty_never_becomes_a_clean_result_or_clears_drain() {
    let d = fixture_dispatch("enable.action0.dispatch");
    let result = dispatch_failure(Verb::Enable, &d.root.intent, ErrorCode::CommitUnknown);
    assert_eq!(result.outcome, Outcome::CommitUnknown);
    assert_eq!(result.exit_code(), 4);
    assert_eq!(
        result.generation.as_deref(),
        Some(d.root.intent.generation.to_string().as_str())
    );
    let replay = ResultWire::new(Verb::Readback, Outcome::Replay, ErrorCode::None);
    let failed = finalize_after_recheck(replay, false, Verb::Readback);
    assert_eq!(failed.outcome, Outcome::CommitUnknown);
    // No readback or custody result can change this production gate.
    assert!(!std::hint::black_box(ENABLE_SQL_RESET_AND_READBACK_READY));
    for verb in [Verb::RestoreOff, Verb::Recover] {
        assert_eq!(
            super::super::super::native_follow_up_block(verb),
            Some(ErrorCode::EvidenceUnavailable)
        );
    }
}

struct Tree {
    path: &'static str,
}
impl Tree {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "hosted Root fixture required"
        );
        assert!(Path::new("/run/r5-paper-checker-integration").is_dir());
        let path = format!(
            "/run/r5-paper-checker-integration/native-action-{}",
            Uuid::new_v4()
        );
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path: Box::leak(path.into_boxed_str()),
        }
    }
    fn path(&self, name: &str) -> String {
        format!("{}/{name}", self.path)
    }
    fn output(&self) -> GuardedDirectory {
        GuardedDirectory::open(self.path).unwrap()
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.path).unwrap();
    }
}

#[test]
#[ignore = "Existing hosted disposable Root fixture; actual fsync/no-replace retained FD/name, no SQL"]
fn actual_bound_fixed_index_prevents_retry_and_rejects_missing_or_replaced_originals() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let tree = Tree::new();
    let output = tree.output();
    let d = fixture_dispatch("configure.action0.dispatch");
    let (first, custody) = persist_bound_index(&output, &d).unwrap();
    assert!(matches!(first, IndexPublication::Created));
    custody.revalidate().unwrap();
    let (again, _) = persist_bound_index(&output, &d).unwrap();
    assert!(matches!(again, IndexPublication::Existing));
    let different = fixture_dispatch("enable.action0.dispatch");
    assert!(persist_bound_index(&output, &different).is_err());
    let root_path = tree.path(&format!("{}.json", &d.root.sha256[7..]));
    let retained_path = tree.path("retained.json");
    std::fs::rename(&root_path, &retained_path).unwrap();
    assert!(RetainedCustody::open(&output, &d).is_err());
    assert!(persist_bound_index(&output, &d).is_err());
    assert!(
        !Path::new(&root_path).exists(),
        "missing original was reconstructed"
    );
    // Recreating identical bytes still changes the original named inode.
    let f = publish_original(&output, d.root.canonical.as_bytes()).unwrap();
    assert!(custody.revalidate().is_err());
    std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(f.revalidate(&output).is_err());
    std::fs::remove_file(&root_path).unwrap();
    symlink(&retained_path, &root_path).unwrap();
    assert!(RetainedCustody::open(&output, &d).is_err());
    drop(custody);
    for name in ["stop.action1.dispatch", "stop.action3.dispatch"] {
        let tree = Tree::new();
        let output = tree.output();
        let dispatch = fixture_dispatch(name);
        let (first, retained) = persist_bound_index(&output, &dispatch).unwrap();
        assert!(matches!(first, IndexPublication::Created));
        let (duplicate, _) = persist_bound_index(&output, &dispatch).unwrap();
        assert!(matches!(duplicate, IndexPublication::Existing));
        retained.revalidate().unwrap();
    }
}

#[test]
#[ignore = "Existing hosted disposable Root fixture; injected failures after each real durable original/index boundary"]
fn actual_bound_publication_fault_boundaries_never_reconstruct_or_resubmit() {
    let d = fixture_dispatch("configure.action0.dispatch");
    // R, A, Q, E, hashed index, fixed index. These are real file/dir fsyncs;
    // callbacks inject a failure only after each completed boundary.
    for stop in 0..6 {
        let tree = Tree::new();
        let output = tree.output();
        let failed = persist_bound_index_at(&output, &d, |n| {
            if n == stop {
                Err(ErrorCode::EvidenceUnavailable)
            } else {
                Ok(())
            }
        });
        assert!(failed.is_err());
        if stop < 5 {
            assert!(!entry_exists(&output, "dispatch.json").unwrap());
            assert!(matches!(
                persist_bound_index(&output, &d),
                Err(ErrorCode::CommitUnknown)
            ));
        } else {
            let (state, custody) = persist_bound_index(&output, &d).unwrap();
            assert!(matches!(state, IndexPublication::Existing));
            custody.revalidate().unwrap();
        }
    }
    for torn in [false, true] {
        let tree = Tree::new();
        let output = tree.output();
        let (_, custody) = persist_bound_index(&output, &d).unwrap();
        std::fs::remove_file(tree.path("dispatch.json")).unwrap();
        if torn {
            // A torn fixed file must never be replaced with a scanned index.
            publish_fixed_index(&output, b"{").unwrap();
        }
        assert!(persist_bound_index(&output, &d).is_err());
        assert!(custody.revalidate().is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "Existing hosted Root/PG fixture; staged bound original ENABLE/v2 custody, expired evidence/later OFF, invalid Current/Daily, no live mounts"]
async fn actual_bound_readback_ignores_live_resources_and_preserves_later_off() {
    let (_, marker) =
        read_fixed("/run/r5-paper-checker-integration/native-bound-case", 128).unwrap();
    assert_eq!(marker, b"readback-bound-offline-committed\n");
    for path in [
        WORKER_PASSWORD,
        CAPABILITY_PATH,
        RESET_SELECTION_PATH,
        RESET_ANCHORS_PATH,
        "/run/owner-beta-paper-ops/operation",
        "/run/owner-beta-paper-ops/provenance",
    ] {
        assert_eq!(
            std::fs::symlink_metadata(path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "{path}"
        );
    }
    let current = crate::owner_beta::paper::source_evidence::UntrustedCurrentEnvelope::decode(
        &std::fs::read("/data/owner-beta-paper-evidence/current/current-binding.json").unwrap(),
    )
    .unwrap();
    assert_eq!(current.status().unwrap(), "INVALID");
    let daily: Value =
        serde_json::from_slice(&std::fs::read("/run/owner-beta-daily/config.json").unwrap())
            .unwrap();
    assert_eq!(daily["enabled"], false);
    let (inputs, intent, _, output, selector) = load_bound_context(Verb::Readback).unwrap();
    let (_, q) = inputs.saved_request.as_ref().unwrap();
    let payload: Value =
        serde_json::from_str(q.value["args"]["evidence_canonical"].as_str().unwrap()).unwrap();
    assert_eq!(q.value["args"]["transition"], "ENABLE");
    assert!(
        chrono::DateTime::parse_from_rfc3339(payload["expires_at"].as_str().unwrap()).unwrap()
            < chrono::Utc::now()
    );
    let result = readback(&intent, &selector, &inputs, &output).await;
    assert_eq!(result.outcome, Outcome::Replay);
    let name = format!("{}.json", &result.artifact_sha256.as_ref().unwrap()[7..]);
    let file = openat_file(
        output.directory.as_raw_fd(),
        &name,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
    .unwrap();
    let rb = ExactReadback::parse(
        serde_json::from_slice(&read_bytes(&file, wire::MAX_EXACT_BYTES).unwrap()).unwrap(),
        q,
    )
    .unwrap();
    assert_eq!(rb.value["current_state"], "OFF");
    assert!(
        rb.value["current_epoch"].as_i64().unwrap() > rb.value["event_epoch"].as_i64().unwrap()
    );
    inputs.revalidate().unwrap();
}


#[test]
fn maintenance_q_uses_original_fence_and_exact_drain_preimage() {
    for name in ["stop.action1.dispatch", "stop.action2.dispatch", "stop.action3.dispatch",
        "backup.action1.dispatch", "backup.action2.dispatch"] {
        let dispatch = fixture_dispatch(name);
        let artifact = dispatch.artifact.as_ref().unwrap();
        let evidence = canonical_text(&artifact.value).unwrap();
        let request = maintenance_request_parts(&dispatch.root, &dispatch.action,
            artifact, evidence.as_bytes()).unwrap();
        assert_eq!(request.canonical, dispatch.request.canonical);
        assert_eq!(request.sha256, dispatch.request.sha256);
        assert_eq!(request.value["args"]["expected_epoch"], dispatch.root.fence_epoch().unwrap());
        if dispatch.action.verb == Verb::Off {
            assert_eq!(artifact.value["expected_epoch"], dispatch.root.intent.expected_epoch);
            assert_eq!(artifact.value["resulting_epoch"], dispatch.root.fence_epoch().unwrap());
            assert_eq!(request.value["args"]["evidence_canonical"], evidence);
        }
        assert!(maintenance_request_parts(&dispatch.root, &dispatch.action, artifact, b"{}").is_err());
        let mut wrong_action = wire::parse_action(dispatch.action.canonical.as_bytes(), &dispatch.root).unwrap();
        wrong_action.expected_epoch += 1;
        assert!(maintenance_request_parts(&dispatch.root, &wrong_action, artifact, evidence.as_bytes()).is_err());
    }
}

#[test]
fn maintenance_result_binds_summary_original_ids_and_distinct_off_epoch() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"))).unwrap();
    let d = fixture_dispatch("stop.action1.dispatch");
    let summary: Value = serde_json::from_str(fixture["objects"]["stop.summary1"]["canonical"].as_str().unwrap()).unwrap();
    let response = json!({"schema":"owner-beta-paper-action-v1", "action":"RECONCILE_OPERATION",
        "outcome":"OK", "control_epoch":1,"account_id":d.root.intent.account_id,
        "data":{"observation_id":summary["observation_id"],"summary":summary,
            "summary_sha256":fixture["objects"]["stop.summary1"]["sha256"]}});
    let artifact = d.artifact.as_ref().unwrap();
    let (_, event, epoch) = maintenance_outcome(&response, Verb::Reconcile, &d.root, artifact).unwrap();
    assert_eq!(event, response["data"]["observation_id"].as_str().unwrap());
    assert_eq!(epoch, 1);
    for field in ["installation_id", "generation", "account_id", "inventory_sha256"] {
        let mut bad = response.clone();
        bad["data"]["summary"][field] = Value::String("substitution".into());
        bad["data"]["summary_sha256"] = Value::String(digest(canonical_text(&bad["data"]["summary"]).unwrap().as_bytes()));
        assert!(maintenance_outcome(&bad, Verb::Reconcile, &d.root, artifact).is_err());
    }
    let mut replay = response.clone();
    replay["outcome"] = json!("REPLAY");
    replay["control_epoch"] = json!(9);
    assert_eq!(maintenance_outcome(&replay, Verb::Reconcile, &d.root, artifact).unwrap().2, 1);
    let off = fixture_dispatch("stop.action3.dispatch");
    let success: Value = serde_json::from_str(fixture["objects"]["stop.result"]["canonical"].as_str().unwrap()).unwrap();
    assert_eq!(maintenance_outcome(&success, Verb::Off, &off.root, off.artifact.as_ref().unwrap()).unwrap().2, 2);
    let mut bad = success.clone();
    bad["control_epoch"] = json!(1);
    assert!(maintenance_outcome(&bad, Verb::Off, &off.root, off.artifact.as_ref().unwrap()).is_err());
    bad = success;
    bad["data"]["state"] = json!("DRAINING");
    assert!(maintenance_outcome(&bad, Verb::Off, &off.root, off.artifact.as_ref().unwrap()).is_err());
}

#[test]
fn maintenance_dispatch_uncertainty_preserves_identity_and_closed_remaining_verbs() {
    for (name, verb, epoch) in [("stop.action1.dispatch", Verb::Reconcile, 1),
        ("stop.action3.dispatch", Verb::Off, 2)] {
        let d = fixture_dispatch(name);
        let pending = dispatch_failure(verb, &d.root.intent, ErrorCode::CommitUnknown);
        assert_eq!(pending.outcome, Outcome::CommitUnknown);
        assert_eq!(pending.exit_code(), 4);
        let mut committed = ResultWire::new(verb, Outcome::Ok, ErrorCode::None);
        committed.generation = Some(d.root.intent.generation.to_string());
        committed.event_id = Some("00000000-0000-4000-8000-000000000099".into());
        committed.control_epoch = Some(epoch);
        committed.artifact_sha256 = Some(d.request.sha256.clone());
        let unknown = finalize_after_recheck(committed, false, verb);
        assert_eq!(unknown.outcome, Outcome::CommitUnknown);
        assert_eq!(unknown.generation.as_deref(), Some(d.root.intent.generation.to_string().as_str()));
        assert_eq!(unknown.event_id.as_deref(), Some("00000000-0000-4000-8000-000000000099"));
        assert_eq!(unknown.control_epoch, Some(epoch));
        assert_eq!(unknown.artifact_sha256.as_deref(), Some(d.request.sha256.as_str()));
    }
    for verb in [Verb::RestoreOff, Verb::Recover] {
        assert_eq!(super::super::super::native_follow_up_block(verb), Some(ErrorCode::EvidenceUnavailable));
    }
}
