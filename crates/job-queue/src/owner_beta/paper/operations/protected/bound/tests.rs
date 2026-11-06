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
    for verb in [Verb::Reconcile, Verb::Off, Verb::RestoreOff, Verb::Recover] {
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
