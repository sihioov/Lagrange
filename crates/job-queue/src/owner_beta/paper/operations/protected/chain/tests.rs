use super::*;
use std::{
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::PathBuf,
};

fn fixture() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json"
    )))
    .unwrap()
}

fn original<'a>(fixture: &'a Value, name: &str) -> &'a [u8] {
    fixture["objects"][name]["canonical"]
        .as_str()
        .unwrap()
        .as_bytes()
}

fn begin_pair(name: &str) -> (RootIntent, super::super::super::wire::ActionBinding) {
    let fixture = fixture();
    let root = wire::parse_root_intent(original(&fixture, &format!("{name}.root"))).unwrap();
    let action = wire::parse_action(original(&fixture, &format!("{name}.action0")), &root).unwrap();
    (root, action)
}

#[test]
fn sequence_zero_live_begin_accepts_only_stop_and_backup_pairs() {
    for name in ["stop", "backup"] {
        let (root, action) = begin_pair(name);
        validate_begin_pair(&root, &action).unwrap();
    }
}

#[test]
fn restore_begin_stays_unavailable_without_archive_proof() {
    let (root, action) = begin_pair("restore");
    assert_eq!(
        validate_begin_pair(&root, &action),
        Err(ErrorCode::EvidenceUnavailable)
    );
}

#[test]
fn intent_stage_parser_rejects_failed_and_later_stage_shapes() {
    let fixture = fixture();
    let (root, _) = begin_pair("stop");
    let root_hash = digest(root.canonical.as_bytes());
    let stage_bytes = original(&fixture, "stop.stage0");
    parse_intent_stage(stage_bytes, &root, &root_hash).unwrap();
    assert!(parse_intent_stage(stage_bytes, &root, &format!("{root_hash}x")).is_err());

    let mut failed: Value = serde_json::from_slice(stage_bytes).unwrap();
    failed["stage"] = Value::String("FAILED".into());
    let failed = super::super::super::super::source_evidence::canonical(&failed).unwrap();
    assert!(parse_intent_stage(&failed, &root, &root_hash).is_err());

    let mut later = serde_json::from_slice::<Value>(stage_bytes).unwrap();
    later["sequence"] = Value::from(1);
    let later = super::super::super::super::source_evidence::canonical(&later).unwrap();
    assert!(parse_intent_stage(&later, &root, &root_hash).is_err());
}

#[test]
fn current_boot_binding_rejects_a_valid_but_stale_boot_uuid() {
    let mut stage = serde_json::json!({"boot_id":local_boot().unwrap().to_string()});
    require_current_boot(&stage).unwrap();
    let stale = uuid::Uuid::from_u128(0x123456789abcdef0123456789abcdef0);
    stage["boot_id"] = Value::String(stale.to_string());
    assert_eq!(
        require_current_boot(&stage),
        Err(ErrorCode::IdentityConflict)
    );
}

#[test]
fn epoch_floor_requires_the_exact_protected_decimal_wire() {
    assert_eq!(parse_epoch_floor(b"0\n"), Ok(0));
    assert_eq!(parse_epoch_floor(b"9223372036854775807\n"), Ok(i64::MAX));
    for invalid in [
        b"".as_slice(),
        b"0",
        b"01\n",
        b"+1\n",
        b"1\r\n",
        b"-1\n",
        b"9223372036854775808\n",
    ] {
        assert!(parse_epoch_floor(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn local_installation_id_requires_canonical_nonnil_uuid_and_one_lf() {
    let (root, _) = begin_pair("stop");
    let expected = root.intent.installation_id;
    let canonical = format!("{expected}\n");
    assert_eq!(parse_installation_id(canonical.as_bytes()), Ok(expected));
    validate_installation_id(canonical.as_bytes(), expected).unwrap();

    for invalid in [
        "",
        "11111111-1111-4111-8111-111111111111",
        "11111111-1111-4111-8111-111111111111\n\n",
        "11111111-1111-4111-8111-11111111111A\n",
        "00000000-0000-0000-0000-000000000000\n",
    ] {
        assert!(
            parse_installation_id(invalid.as_bytes()).is_err(),
            "accepted {invalid:?}"
        );
    }
    let other = uuid::Uuid::from_u128(expected.as_u128() ^ 1);
    assert_eq!(
        validate_installation_id(format!("{other}\n").as_bytes(), expected),
        Err(ErrorCode::IdentityConflict)
    );
}

#[test]
fn stale_floor_and_action_generation_substitutions_reject() {
    let (root, action) = begin_pair("stop");
    let mut stale = root.intent.value.clone();
    stale["local_epoch_floor"] = Value::from(root.intent.local_epoch_floor + 1);
    let stale = super::super::super::super::source_evidence::canonical(&stale).unwrap();
    assert!(wire::parse_root_intent(&stale).is_err());

    let fixture = fixture();
    let mut later: Value = serde_json::from_slice(original(&fixture, "stop.action0")).unwrap();
    later["sequence"] = Value::from(1);
    let later = super::super::super::super::source_evidence::canonical(&later).unwrap();
    assert!(wire::parse_action(&later, &root).is_err());
    validate_begin_pair(&root, &action).unwrap();
}

#[test]
fn fixed_stage_and_reserved_action_entries_reject_gap_fork_and_unknown_names() {
    let expected_root = BTreeSet::from([
        "intent.json".to_owned(),
        "00000000.json".to_owned(),
        "actions".to_owned(),
        format!("{}.json", "a".repeat(64)),
    ]);
    require_exact_names(expected_root.clone(), expected_root.clone()).unwrap();
    for extra in [
        "00000002.json",
        "00000001.json",
        ".00000001.json.tmp",
        "scan.json",
    ] {
        let mut actual = expected_root.clone();
        actual.insert(extra.to_owned());
        assert_eq!(
            require_exact_names(actual, expected_root.clone()),
            Err(ErrorCode::IdentityConflict),
            "accepted unexpected operation entry {extra}"
        );
    }
    let expected_actions = BTreeSet::from(["00000000".to_owned()]);
    let with_later_action = BTreeSet::from(["00000000".to_owned(), "00000001".to_owned()]);
    assert_eq!(
        require_exact_names(with_later_action, expected_actions),
        Err(ErrorCode::IdentityConflict)
    );
}

struct RootPrivateDirectory {
    path: PathBuf,
    directory: std::fs::File,
    identity: Identity,
}

impl Drop for RootPrivateDirectory {
    fn drop(&mut self) {
        let Ok(opened_identity) = identity(&self.directory) else {
            return;
        };
        let Ok(named) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if !same_object(opened_identity, self.identity)
            || !named.file_type().is_dir()
            || named.dev() != self.identity.device
            || named.ino() != self.identity.inode
            || named.uid() != 0
            || named.gid() != 0
            || (named.mode() & 0o7777) != 0o700
        {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return;
            };
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                return;
            };
            if !metadata.file_type().is_file()
                || metadata.uid() != 0
                || metadata.gid() != 0
                || metadata.nlink() != 1
                || !matches!(metadata.mode() & 0o7777, 0o400 | 0o600)
            {
                return;
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn root_private_test_directory(label: &str) -> (RootPrivateDirectory, &'static str) {
    assert_eq!(unsafe { libc::geteuid() }, 0, "Root-only filesystem unit");
    assert_eq!(unsafe { libc::getegid() }, 0, "root:root filesystem unit");
    assert!(matches!(
        label,
        "native-chain-state" | "native-chain-result"
    ));
    assert!(std::path::Path::new("/run/r5-paper-checker-integration").is_dir());
    let path = PathBuf::from(format!(
        "/run/r5-paper-checker-integration/{label}-{}",
        uuid::Uuid::new_v4()
    ));
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(&path).unwrap();
    let fixed_path: &'static str = Box::leak(path.to_string_lossy().into_owned().into_boxed_str());
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&path)
        .unwrap();
    let identity = identity(&directory).unwrap();
    (
        RootPrivateDirectory {
            path,
            directory,
            identity,
        },
        fixed_path,
    )
}

fn write_private_test_file(path: &std::path::Path, bytes: &[u8]) {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(path).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn assert_committed_result_is_unknown(
    result: ResultWire,
    recheck_succeeded: bool,
    expected_digest: &str,
) {
    let unresolved = super::super::finalize_after_recheck(result, recheck_succeeded, Verb::Begin);
    assert_eq!(unresolved.outcome, Outcome::CommitUnknown);
    assert_eq!(unresolved.error_code, ErrorCode::CommitUnknown);
    assert_eq!(
        unresolved.generation.as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(
        unresolved.event_id.as_deref(),
        Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(unresolved.control_epoch, Some(42));
    assert_eq!(unresolved.artifact_sha256.as_deref(), Some(expected_digest));
}

#[test]
#[ignore = "Root-only private filesystem unit; creates genuine root-owned 0700/0600 OS objects in a disposable directory, not a real install identity or BEGIN permit"]
fn retained_paper_state_checks_identity_floor_missing_mismatch_and_name_replacement() {
    let (directory, directory_path) = root_private_test_directory("native-chain-state");
    let installation_path: &'static str = Box::leak(
        directory
            .path
            .join("installation-id")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    );
    let floor_path: &'static str = Box::leak(
        directory
            .path
            .join("epoch-floor")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    );
    let installation_id = uuid::Uuid::new_v4();
    let installation_bytes = format!("{installation_id}\n").into_bytes();
    let floor_bytes = b"41\n";
    write_private_test_file(std::path::Path::new(floor_path), floor_bytes);
    assert_eq!(
        RetainedPaperState::open(
            directory_path,
            installation_path,
            floor_path,
            installation_id,
            41
        )
        .err(),
        Some(ErrorCode::EvidenceUnavailable)
    );

    write_private_test_file(std::path::Path::new(installation_path), &installation_bytes);
    assert_eq!(
        RetainedPaperState::open(
            directory_path,
            installation_path,
            floor_path,
            uuid::Uuid::new_v4(),
            41
        )
        .err(),
        Some(ErrorCode::IdentityConflict)
    );
    assert_eq!(
        RetainedPaperState::open(
            directory_path,
            installation_path,
            floor_path,
            installation_id,
            42
        )
        .err(),
        Some(ErrorCode::StaleEpoch)
    );

    let mut retained = RetainedPaperState::open(
        directory_path,
        installation_path,
        floor_path,
        installation_id,
        41,
    )
    .unwrap();
    retained.revalidate(installation_id, 41).unwrap();
    std::fs::remove_file(installation_path).unwrap();
    write_private_test_file(std::path::Path::new(installation_path), &installation_bytes);
    assert_eq!(
        retained.revalidate(installation_id, 41),
        Err(ErrorCode::ReaderIncompatible)
    );

    let mut retained_after_installation_replacement = RetainedPaperState::open(
        directory_path,
        installation_path,
        floor_path,
        installation_id,
        41,
    )
    .unwrap();
    std::fs::remove_file(floor_path).unwrap();
    write_private_test_file(std::path::Path::new(floor_path), floor_bytes);
    assert_eq!(
        retained_after_installation_replacement.revalidate(installation_id, 41),
        Err(ErrorCode::ReaderIncompatible)
    );
    check_protected_inventory_closure(&directory, directory_path);
}

#[test]
#[ignore = "Root-only private output filesystem unit; synthetic canonical bytes exercise post-COMMIT custody classification only, not SQL or a genuine BEGIN result"]
fn retained_post_commit_result_loss_or_same_byte_replacement_preserves_unknown_identity() {
    let (directory, directory_path) = root_private_test_directory("native-chain-result");
    let output = GuardedDirectory::open(directory_path).unwrap();
    let bytes = super::super::super::super::source_evidence::canonical(
        &serde_json::json!({"fixture":"filesystem custody only"}),
    )
    .unwrap();
    let original = output.write_content_addressed_retained(&bytes).unwrap();
    let leaf = original.leaf.clone();
    let relative = RelativeOriginal::open(&output, vec![leaf.clone()], wire::MAX_WIRE_BYTES).unwrap();
    relative.revalidate(&output).unwrap();
    let first_digest = original.digest.clone();
    let first_inode = original.identity.inode;
    original.revalidate(&output).unwrap();

    std::fs::remove_file(directory.path.join(&leaf)).unwrap();
    let deleted_recheck_succeeded = original.revalidate(&output).is_ok();
    assert!(!deleted_recheck_succeeded, "deleted result remained valid");
    assert!(relative.revalidate(&output).is_err());
    let committed = || {
        let mut result = ResultWire::new(Verb::Begin, Outcome::Ok, ErrorCode::None);
        result.generation = Some("11111111-1111-4111-8111-111111111111".into());
        result.event_id = Some("22222222-2222-4222-8222-222222222222".into());
        result.control_epoch = Some(42);
        result.artifact_sha256 = Some(first_digest.clone());
        result
    };
    assert_committed_result_is_unknown(committed(), deleted_recheck_succeeded, &first_digest);

    let replacement = output.write_content_addressed_retained(&bytes).unwrap();
    assert_ne!(first_inode, replacement.identity.inode);
    assert_eq!(first_digest, replacement.digest);
    assert!(relative.revalidate(&output).is_err(), "relative same-byte substitution accepted");
    let replacement_recheck_succeeded = original.revalidate(&output).is_ok();
    assert!(
        !replacement_recheck_succeeded,
        "same-byte replacement became the retained original"
    );
    assert_committed_result_is_unknown(committed(), replacement_recheck_succeeded, &first_digest);
    replacement.revalidate(&output).unwrap();
}


fn maintenance_stage_chain(name: &str, last: i64) -> (RootIntent, Vec<Value>, Vec<String>) {
    let fixture = fixture();
    let (root, _) = begin_pair(name);
    let mut stages: Vec<Value> = Vec::new();
    let mut hashes: Vec<String> = Vec::new();
    for sequence in 0..=last {
        let bytes = original(&fixture, &format!("{name}.stage{sequence}"));
        let stage = if sequence == 0 {
            parse_intent_stage(bytes, &root, &root.sha256).unwrap()
        } else {
            stage_after(bytes, &root, sequence, stages.last().unwrap(), hashes.last().unwrap()).unwrap()
        };
        stages.push(stage);
        hashes.push(digest(bytes));
    }
    (root, stages, hashes)
}

#[test]
fn retained_stage_mapping_accepts_stop_and_backup_reconcile_then_off() {
    let fixture = fixture();
    for (name, last) in [("stop", 4), ("backup", 3)] {
        let (root, stages, hashes) = maintenance_stage_chain(name, last);
        for sequence in 1..last {
            let action = wire::parse_action(original(&fixture, &format!("{name}.action{sequence}")), &root).unwrap();
            let index = usize::try_from(sequence + 1).unwrap();
            maintenance_pair(&root, &action, &stages[index], &hashes[index]).unwrap();
            let mut bad = stages[index].clone();
            bad["stage"] = json!("STOPPED");
            if action.verb == Verb::Off {
                assert_eq!(maintenance_pair(&root, &action, &bad, &hashes[index]), Err(ErrorCode::EvidenceUnavailable));
            }
            assert!(maintenance_pair(&root, &action, &stages[index], &hashes[0]).is_err());
        }
    }
}

#[test]
fn sealed_stage_rejects_mixed_identity_boot_fence_event_order_and_missing_sequence() {
    let fixture = fixture();
    let (root, stages, hashes) = maintenance_stage_chain("stop", 4);
    let good: Value = serde_json::from_slice(original(&fixture, "stop.stage2")).unwrap();
    for (field, value) in [
        ("installation_id", json!("00000000-0000-4000-8000-000000000099")),
        ("generation", json!("00000000-0000-4000-8000-000000000099")),
        ("account_id", json!("00000000-0000-4000-8000-000000000099")),
        ("boot_id", json!("00000000-0000-4000-8000-000000000099")),
        ("event_id", json!("00000000-0000-4000-8000-000000000099")),
        ("sequence", json!(3)), ("control_epoch", json!(2)),
        ("previous_sha256", json!(root.sha256)), ("intent_sha256", json!(hashes[1])),
        ("stage", json!("RECONCILED")), ("error_code", json!("WRITER_UNKNOWN")),
    ] {
        let mut bad = good.clone();
        bad[field] = value;
        assert!(stage_after(canonical_text(&bad).unwrap().as_bytes(), &root, 2, &stages[1], &hashes[1]).is_err(), "accepted {field}");
    }
    for clock in ["realtime_ns", "monotonic_ns", "boottime_ns"] {
        let mut bad = good.clone();
        bad["observed"][clock] = json!(0);
        assert!(stage_after(canonical_text(&bad).unwrap().as_bytes(), &root, 2, &stages[1], &hashes[1]).is_err());
    }
    assert!(stage_after(original(&fixture, "stop.stage4"), &root, 3, &stages[2], &hashes[2]).is_err());
    let action = wire::parse_action(original(&fixture, "stop.action1"), &root).unwrap();
    assert!(maintenance_pair(&root, &action, &stages[1], &hashes[1]).is_err());
}

fn backend_original_fixture(root: &RootIntent, fence: &Value) -> Value {
    let row = json!({"pid":321,"start_us":1793923210000000_i64,"role":"app","addr":"172.18.0.3","port":45678});
    let observer = json!({"pid":99,"user":"postgres","session":"postgres","read_only":"on","role":"none","superuser":true});
    let state = json!({"epoch":root.fence_epoch().unwrap(),"state":"DRAINING",
        "event":fence["event_id"],"generation":root.intent.generation});
    let mut captured = row.clone();
    captured["process"] = json!({"pid":123,"start_ticks":123456,"owner":["CONTAINER","a".repeat(64)]});
    captured["socket"] = json!({"pid":123,"start_ticks":123456,"socket_inode":555,
        "local":["172.18.0.3",45678],"peer":["172.18.0.2",5432]});
    json!({"captured":[captured],
        "initial":{"observer":observer,"prepared":0,"unexpected":0,"backends":[row],"fence":state},
        "final":{"observer":observer,"prepared":0,"unexpected":0,"backends":[],"fence":state},
        "database_generation":["b".repeat(64),"2026-11-06T00:00:00.000001Z",
            "2026-11-06T00:00:01.000001Z",0,format!("sha256:{}", "c".repeat(64))]})
}

#[test]
fn backend_proof_requires_exact_pid_start_socket_observer_and_no_prepared_work() {
    let (root, stages, _) = maintenance_stage_chain("stop", 2);
    let good = backend_original_fixture(&root, &stages[1]);
    assert_eq!(validate_backend_original(&good, &root, &stages[1]).unwrap(), vec![321]);
    for pointer in ["/final/prepared", "/initial/prepared", "/final/unexpected"] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = json!(1);
        assert!(validate_backend_original(&bad, &root, &stages[1]).is_err());
    }
    for (pointer, value) in [
        ("/final/backends", good["initial"]["backends"].clone()),
        ("/initial/backends/0/start_us", json!(1)),
        ("/captured/0/pid", json!(322)), ("/captured/0/socket/start_ticks", json!(1)),
        ("/captured/0/socket/socket_inode", json!(0)), ("/captured/0/socket/peer/1", json!(1234)),
        ("/captured/0/socket/local/1", json!(1111)),
        ("/final/observer/read_only", json!("off")), ("/initial/observer/user", json!("migration_owner")),
        ("/final/observer/role", json!("app")), ("/initial/observer/superuser", json!(false)),
        ("/final/fence/event", json!("00000000-0000-4000-8000-000000000099")),
        ("/final/fence/epoch", json!(2)), ("/database_generation/3", json!(-1)),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(validate_backend_original(&bad, &root, &stages[1]).is_err(), "accepted {pointer}");
    }
    let mut duplicate = good.clone();
    duplicate["captured"].as_array_mut().unwrap().push(good["captured"][0].clone());
    duplicate["initial"]["backends"].as_array_mut().unwrap().push(good["initial"]["backends"][0].clone());
    assert!(validate_backend_original(&duplicate, &root, &stages[1]).is_err());
}

#[test]
fn prior_committed_readback_requires_original_success_not_replay_or_current_authority() {
    let fixture = fixture();
    let dispatch = wire::BoundDispatch::parse(original(&fixture, "stop.action1.dispatch")).unwrap();
    let summary: Value = serde_json::from_slice(original(&fixture, "stop.summary1")).unwrap();
    let canonical = canonical_text(&json!({"schema":"owner-beta-paper-action-v1",
        "action":"RECONCILE_OPERATION","outcome":"OK","control_epoch":1,
        "account_id":dispatch.root.intent.account_id,"data":{"observation_id":summary["observation_id"],
            "summary":summary,"summary_sha256":digest(original(&fixture, "stop.summary1"))}})).unwrap();
    let event = summary["observation_id"].as_str().unwrap();
    let readback = json!({"schema":"owner-beta-paper-exact-readback-v1","kind":"RECONCILE",
        "request_sha256":dispatch.request.sha256,"status":"COMMITTED","account_id":dispatch.root.intent.account_id,
        "current_epoch":1,"current_state":"DRAINING","event_id":event,"event_epoch":1,
        "result_canonical":canonical,"result_sha256":digest(canonical.as_bytes()),"reset":null});
    let prior = PriorMaintenanceResult { request:dispatch.request, canonical:canonical.clone(),event_id:event.into(),epoch:1 };
    let exact = ExactReadback::parse(readback.clone(), &prior.request).unwrap();
    prior.verify_readback(&exact, 1).unwrap();
    for (key, value) in [("current_state",json!("OFF")),("current_epoch",json!(2))] {
        let mut bad = readback.clone();
        bad[key] = value;
        let historical = ExactReadback::parse(bad, &prior.request).unwrap();
        assert_eq!(prior.verify_readback(&historical, 1), Err(ErrorCode::CommitUnknown));
    }
    let mut changed = readback.clone();
    changed["event_id"] = json!("00000000-0000-4000-8000-000000000099");
    assert!(ExactReadback::parse(changed, &prior.request).is_err());
    let mut replay: Value = serde_json::from_str(&canonical).unwrap();
    replay["outcome"] = json!("REPLAY");
    let replay = canonical_text(&replay).unwrap();
    let mut changed = readback;
    changed["result_canonical"] = json!(replay);
    changed["result_sha256"] = json!(digest(replay.as_bytes()));
    assert!(ExactReadback::parse(changed, &prior.request).is_err());
}

#[test]
fn observation_graph_follows_original_links_not_descriptive_image_or_elf_hashes() {
    let hash = format!("sha256:{}", "a".repeat(64));
    let descriptor = format!("sha256:{}", "b".repeat(64));
    let mut links = BTreeSet::new();
    collect_observation_links(&json!({"identity_sha256":hash,"image":descriptor,
        "api_binary_sha256":descriptor,"containers":{"id":hash},"processes":[hash]}), &mut links).unwrap();
    assert_eq!(links, BTreeSet::from([hash]));
    assert!(digest_leaf("../../arbitrary").is_err());
    assert!(digest_leaf(&format!("sha256:{}", "A".repeat(64))).is_err());
}

// Filesystem/closed-graph component vectors only. This never constructs
// LiveMaintenanceInputs or substitutes for the selected ELF / genuine observer.
fn check_protected_inventory_closure(directory: &RootPrivateDirectory, directory_path: &'static str) {
    let (root, stages, _) = maintenance_stage_chain("stop", 2);
    let put = |value: &Value| {
        let bytes = canonical_text(value).unwrap().into_bytes();
        let hash = digest(&bytes);
        let path = directory.path.join(digest_leaf(&hash).unwrap());
        if !path.exists() {
            write_private_test_file(&path, &bytes);
        }
        hash
    };
    let mut backend = backend_original_fixture(&root, &stages[1]);
    backend["captured"] = json!([]);
    backend["initial"]["backends"] = json!([]);
    let backend_hash = put(&backend);
    let launch = json!({"units":{},"containers":{},"pending_owned_jobs":[]});
    let launch_hash = put(&launch);
    let observed = json!({"resources":[],"launches":launch});
    let capture = json!({"boot_id":stages[1]["boot_id"],"manifest":root.intent.release_manifest_sha256,
        "containers":{},"units":{},"processes":[],"writable_roots":{},"other_launch_sources":{},
        "backend_observation":backend["initial"]});
    let capture_hash = put(&capture);
    let mut rows = Vec::new();
    for class in ["API", "PAPER_RUNNER", "PAPER_DELIVERY"] {
        let identity = json!({"writer_class":class,"installation_id":root.intent.installation_id,
            "generation":root.intent.generation,"boot_id":stages[1]["boot_id"],
            "manifest":root.intent.release_manifest_sha256,"capture_sha256":capture_hash});
        let identity_hash = put(&identity);
        let observation_hash = put(&json!({"identity_sha256":identity_hash,"first":observed,
            "second":observed,"launch_fence_sha256":launch_hash,"backend_observation_sha256":backend_hash}));
        rows.push(json!({"writer_class":class,"kind":"UNIT","identity_sha256":identity_hash,
            "observation_sha256":observation_hash,"backend_set_sha256":backend_hash,"terminal":"ABSENT_AT_FENCE"}));
    }
    rows.sort_by(|a, b| a["identity_sha256"].as_str().cmp(&b["identity_sha256"].as_str()));
    let inventory = json!({"schema":"owner-beta-paper-writer-inventory-v1",
        "installation_id":root.intent.installation_id,"generation":root.intent.generation,
        "account_id":root.intent.account_id,"scope":"PAPER_STOP","fence_event_id":stages[1]["event_id"],
        "control_epoch":1,"boot_id":stages[1]["boot_id"],"observed":stages[2]["observed"],
        "release_manifest_sha256":root.intent.release_manifest_sha256,"writers":rows,
        "launch_fence_sha256":launch_hash,"backend_observation_sha256":backend_hash});
    let mut tree = MaintenanceTree { root:GuardedDirectory::open(directory_path).unwrap(),
        files:std::collections::BTreeMap::new(),directories:std::collections::BTreeMap::new() };
    assert!(validate_writer_originals(&mut tree, &root, &inventory, &stages[1]).unwrap().is_empty());
    tree.revalidate().unwrap();
    let mut missing = inventory.clone();
    missing["writers"].as_array_mut().unwrap().pop();
    assert!(validate_writer_originals(&mut tree, &root, &missing, &stages[1]).is_err());
    let mut mixed = inventory.clone();
    mixed["fence_event_id"] = json!("00000000-0000-4000-8000-000000000099");
    assert!(validate_writer_originals(&mut tree, &root, &mixed, &stages[1]).is_err());
    let mut unknown = inventory.clone();
    unknown["writers"][0]["terminal"] = json!("UNKNOWN");
    assert!(validate_writer_originals(&mut tree, &root, &unknown, &stages[1]).is_err());
    // A captured unit with three classes shares one exact generation/backend
    // set. These are protected component bytes, never claimed host observations.
    let unit = json!({"name":"fixture.service","pid":123,"cgroup":"/fixture.service",
        "classes":["API","PAPER_RUNNER","PAPER_DELIVERY"],"originals":[],
        "source_config":format!("sha256:{}", "d".repeat(64)),"triggers":[],"triggered_by":[]});
    let unit_hash = put(&unit);
    let mut disabled = unit.clone();
    disabled["pid"] = json!(0);
    disabled["active"] = json!("inactive");
    disabled["file_state"] = json!("masked");
    let disabled_hash = put(&disabled);
    let live_launch = json!({"units":{"fixture.service":disabled_hash},"containers":{},"pending_owned_jobs":[]});
    let live_launch_hash = put(&live_launch);
    let live_identity = json!({"kind":"UNIT","generation":unit,"boot_id":stages[1]["boot_id"],
        "manifest":root.intent.release_manifest_sha256});
    let live_identity_hash = put(&live_identity);
    let terminal = json!({"resources":[{"kind":"UNIT","identity_sha256":unit_hash,
        "cgroup":{"identity":[1,2,0,0,493,2],"removed":false,"populated":0}}],"launches":live_launch});
    let observation = json!({"identity_sha256":live_identity_hash,"first":terminal,"second":terminal});
    let observation_hash = put(&observation);
    let mut with_backend = backend_original_fixture(&root, &stages[1]);
    with_backend["captured"][0]["process"]["owner"] = json!(["UNIT","fixture.service"]);
    let with_backend_hash = put(&with_backend);
    let set = json!({"identity_sha256":live_identity_hash,"backend_observation_sha256":with_backend_hash,
        "backends":with_backend["captured"]});
    let set_hash = put(&set);
    let mut with_unit = inventory.clone();
    let writer_rows: Vec<_> = ["API", "PAPER_DELIVERY", "PAPER_RUNNER"].into_iter().map(|class|
        json!({"writer_class":class,"kind":"UNIT","identity_sha256":live_identity_hash,
            "observation_sha256":observation_hash,"backend_set_sha256":set_hash,"terminal":"EXITED_REAPED"})).collect();
    with_unit["writers"] = Value::Array(writer_rows);
    with_unit["launch_fence_sha256"] = json!(live_launch_hash);
    with_unit["backend_observation_sha256"] = json!(with_backend_hash);
    assert_eq!(validate_writer_originals(&mut tree, &root, &with_unit, &stages[1]).unwrap(), vec![321]);
    for (pointer, changed) in [("/second/resources/0/cgroup/populated",json!(1)),
        ("/second/resources/0/cgroup/identity",json!([9,9,0,0,493,2])),
        ("/first/resources",json!([]))] {
        let mut bad_observation = observation.clone();
        *bad_observation.pointer_mut(pointer).unwrap() = changed;
        let bad_hash = put(&bad_observation);
        let mut bad = with_unit.clone();
        for row in bad["writers"].as_array_mut().unwrap() {
            row["observation_sha256"] = json!(bad_hash);
        }
        assert!(validate_writer_originals(&mut tree, &root, &bad, &stages[1]).is_err());
    }
    let mut empty_set = set;
    empty_set["backends"] = json!([]);
    let empty_hash = put(&empty_set);
    let mut unassigned = with_unit;
    for row in unassigned["writers"].as_array_mut().unwrap() {
        row["backend_set_sha256"] = json!(empty_hash);
    }
    assert!(validate_writer_originals(&mut tree, &root, &unassigned, &stages[1]).is_err());
    let generation = json!({"Id":"a".repeat(64),"Image":format!("sha256:{}", "b".repeat(64)),
        "Created":"2026-11-06T00:00:00.000001Z","StartedAt":"2026-11-06T00:00:01.000001Z","RestartCount":0});
    let captured_container = json!({"generation":generation,"pid":123,"cgroup":"/fixture.container",
        "classes":["API"],"mounts":[],"policy":{},"restart":"no"});
    let captured_hash = put(&captured_container);
    let mut exited_container = captured_container.clone();
    exited_container["pid"] = json!(0);
    let exited_hash = put(&exited_container);
    let mut container_map = serde_json::Map::new();
    container_map.insert("a".repeat(64), json!(exited_hash));
    let container_launch = json!({"units":{},"containers":container_map,"pending_owned_jobs":[]});
    let container_identity = json!({"kind":"CONTAINER","generation":captured_container,
        "boot_id":stages[1]["boot_id"],"manifest":root.intent.release_manifest_sha256});
    let container_observation = json!({"launches":container_launch,"resources":[{
        "kind":"CONTAINER","identity_sha256":captured_hash,
        "cgroup":{"identity":[1,2,0,0,493,2],"removed":true},
        "terminal":{"generation":generation,"restart":"no","exit_code":0,
            "finished_at":"2026-11-06T00:00:02.000001Z"}}]});
    validate_terminal_original(&mut tree, &container_observation, &container_identity).unwrap();
    validate_launch_binding(&mut tree, &container_launch, &container_identity).unwrap();
    for (pointer, value) in [("/resources/0/terminal/generation/RestartCount",json!(1)),
        ("/resources/0/terminal/restart",json!("always")),
        ("/resources/0/terminal/finished_at",json!("2026-11-05T00:00:00Z")),
        ("/resources/0/cgroup",Value::Null)] {
        let mut changed = container_observation.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(validate_terminal_original(&mut tree, &changed, &container_identity).is_err());
    }
    exited_container["generation"]["RestartCount"] = json!(1);
    let substituted_hash = put(&exited_container);
    let mut changed_launch = container_launch;
    changed_launch["containers"]["a".repeat(64)] = json!(substituted_hash);
    assert!(validate_launch_binding(&mut tree, &changed_launch, &container_identity).is_err());
    let backend_path = directory.path.join(digest_leaf(&backend_hash).unwrap());
    std::fs::remove_file(&backend_path).unwrap();
    assert!(validate_writer_originals(&mut tree, &root, &inventory, &stages[1]).is_err());
    write_private_test_file(&backend_path, canonical_text(&backend).unwrap().as_bytes());
    assert!(validate_writer_originals(&mut tree, &root, &inventory, &stages[1]).is_err(),
        "same-byte replacement became an original backend observation");
}
