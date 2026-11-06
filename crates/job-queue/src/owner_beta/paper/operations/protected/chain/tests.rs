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
    let first_digest = original.digest.clone();
    let first_inode = original.identity.inode;
    original.revalidate(&output).unwrap();

    std::fs::remove_file(directory.path.join(&leaf)).unwrap();
    let deleted_recheck_succeeded = original.revalidate(&output).is_ok();
    assert!(!deleted_recheck_succeeded, "deleted result remained valid");
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
    let replacement_recheck_succeeded = original.revalidate(&output).is_ok();
    assert!(
        !replacement_recheck_succeeded,
        "same-byte replacement became the retained original"
    );
    assert_committed_result_is_unknown(committed(), replacement_recheck_succeeded, &first_digest);
    replacement.revalidate(&output).unwrap();
}
