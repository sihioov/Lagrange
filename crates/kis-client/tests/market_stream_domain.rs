#![cfg(feature = "test-support")]

use std::fs::{self, Permissions};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::time::{Duration, Instant};

use kis_client::market_stream_state::{
    CONNECTION_LOCK_FILE, MarketStreamDomain, STATE_FILE, STATE_LOCK_FILE, StateError,
};
use uuid::Uuid;

fn paths(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    (
        root.join("kis-market-stream"),
        root.join("kis-market-stream-locks"),
    )
}

#[test]
fn synthetic_domain_has_exact_layout_and_reopens_without_reset() {
    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let (state, anchors) = paths(root.path());
    assert_eq!(fs::metadata(&state).unwrap().mode() & 0o7777, 0o700);
    assert_eq!(fs::metadata(&anchors).unwrap().mode() & 0o7777, 0o750);
    assert_eq!(
        fs::metadata(state.join(STATE_FILE)).unwrap().mode() & 0o7777,
        0o600
    );
    let connection = fs::metadata(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    let state_lock = fs::metadata(anchors.join(STATE_LOCK_FILE)).unwrap();
    for metadata in [&connection, &state_lock] {
        assert_eq!(metadata.mode() & 0o7777, 0o440);
        assert_eq!(metadata.len(), 0);
        assert_eq!(metadata.nlink(), 1);
    }
    assert_ne!(
        (connection.dev(), connection.ino()),
        (state_lock.dev(), state_lock.ino())
    );
    let before = fs::read(domain.state_directory().join(STATE_FILE)).unwrap();
    MarketStreamDomain::for_test(root.path(), slot).unwrap();
    assert_eq!(
        before,
        fs::read(domain.state_directory().join(STATE_FILE)).unwrap()
    );
}

#[test]
fn existing_uninitialized_and_partial_layouts_fail_without_change() {
    let root = tempfile::tempdir().unwrap();
    let (state, anchors) = paths(root.path());
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&anchors).unwrap();
    fs::set_permissions(&anchors, Permissions::from_mode(0o750)).unwrap();
    for name in [CONNECTION_LOCK_FILE, STATE_LOCK_FILE] {
        fs::File::create(anchors.join(name)).unwrap();
        fs::set_permissions(anchors.join(name), Permissions::from_mode(0o440)).unwrap();
    }
    let connection = fs::metadata(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    let state_lock = fs::metadata(anchors.join(STATE_LOCK_FILE)).unwrap();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap_err(),
        StateError::PriorSessionUncertain
    );
    assert!(!state.join(STATE_FILE).exists());
    assert_eq!(
        connection.ino(),
        fs::metadata(anchors.join(CONNECTION_LOCK_FILE))
            .unwrap()
            .ino()
    );
    assert_eq!(
        state_lock.ino(),
        fs::metadata(anchors.join(STATE_LOCK_FILE)).unwrap().ino()
    );

    let root = tempfile::tempdir().unwrap();
    let (state, anchors) = paths(root.path());
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, Permissions::from_mode(0o700)).unwrap();
    let state_inode = fs::metadata(&state).unwrap().ino();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap_err(),
        StateError::UnsafePath
    );
    assert_eq!(state_inode, fs::metadata(&state).unwrap().ino());
    assert!(!anchors.exists());
    assert!(!state.join(STATE_FILE).exists());
}

#[test]
fn every_connection_acquisition_uses_a_contending_open_description() {
    let root = tempfile::tempdir().unwrap();
    let domain = MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap();
    let first = domain.test_lock().unwrap();
    assert_eq!(domain.test_lock().unwrap_err(), StateError::LockBusy);
    drop(first);
    assert!(domain.test_lock().is_ok());
}

#[test]
fn held_state_anchor_is_promptly_busy_without_mutation_then_fresh_fd_succeeds() {
    let root = tempfile::tempdir().unwrap();
    let domain = MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    let before = fs::read(&state_path).unwrap();
    let held = domain.test_state_lock().unwrap();
    let started = Instant::now();
    assert_eq!(
        domain.test_append_command_attempt(10_000).unwrap_err(),
        StateError::LockBusy
    );
    assert!(started.elapsed() < Duration::from_millis(250));
    assert_eq!(before, fs::read(&state_path).unwrap());
    drop(held);
    domain.test_append_command_attempt(10_001).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let attempts = json["command_attempts_ms"].as_array().unwrap();
    assert_eq!(attempts, &[serde_json::json!(10_001)]);
}

#[test]
fn nonwritable_anchor_parent_denies_ordinary_rename_and_unlink() {
    let root = tempfile::tempdir().unwrap();
    let domain = MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap();
    let (_, anchors) = paths(root.path());
    fs::set_permissions(&anchors, Permissions::from_mode(0o550)).unwrap();
    assert!(fs::rename(anchors.join(CONNECTION_LOCK_FILE), anchors.join("moved")).is_err());
    assert!(fs::remove_file(anchors.join(STATE_LOCK_FILE)).is_err());
    fs::set_permissions(&anchors, Permissions::from_mode(0o750)).unwrap();
    assert!(domain.test_lock().is_ok());
}

fn assert_reopen_fails_after(mutator: impl FnOnce(&std::path::Path, &std::path::Path)) {
    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let before = fs::read(domain.state_directory().join(STATE_FILE)).unwrap();
    let (state, anchors) = paths(root.path());
    mutator(&state, &anchors);
    assert!(MarketStreamDomain::for_test(root.path(), slot).is_err());
    assert_eq!(
        before,
        fs::read(domain.state_directory().join(STATE_FILE)).unwrap()
    );
}

#[test]
fn anchor_metadata_type_mode_link_size_symlink_missing_and_same_inode_fail_closed() {
    assert_reopen_fails_after(|_, anchors| {
        fs::set_permissions(
            anchors.join(CONNECTION_LOCK_FILE),
            Permissions::from_mode(0o600),
        )
        .unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::set_permissions(
            anchors.join(CONNECTION_LOCK_FILE),
            Permissions::from_mode(0o640),
        )
        .unwrap();
        fs::write(anchors.join(CONNECTION_LOCK_FILE), b"x").unwrap();
        fs::set_permissions(
            anchors.join(CONNECTION_LOCK_FILE),
            Permissions::from_mode(0o440),
        )
        .unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::hard_link(anchors.join(CONNECTION_LOCK_FILE), anchors.join("extra")).unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::remove_file(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
        fs::create_dir(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::remove_file(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
        symlink(
            anchors.join(STATE_LOCK_FILE),
            anchors.join(CONNECTION_LOCK_FILE),
        )
        .unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::remove_file(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    });
    assert_reopen_fails_after(|_, anchors| {
        fs::remove_file(anchors.join(STATE_LOCK_FILE)).unwrap();
        fs::hard_link(
            anchors.join(CONNECTION_LOCK_FILE),
            anchors.join(STATE_LOCK_FILE),
        )
        .unwrap();
    });
}

#[test]
fn slot_identity_and_legacy_state_mismatches_do_not_mutate_state() {
    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    let before = fs::read(&state_path).unwrap();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap_err(),
        StateError::InvalidState
    );
    assert_eq!(before, fs::read(&state_path).unwrap());

    let root = tempfile::tempdir().unwrap();
    let domain = MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    fs::write(&state_path, br#"{"schema":"kis-market-stream-state-v1"}"#).unwrap();
    fs::set_permissions(&state_path, Permissions::from_mode(0o600)).unwrap();
    let legacy = fs::read(&state_path).unwrap();
    assert!(MarketStreamDomain::for_test(root.path(), Uuid::new_v4()).is_err());
    assert_eq!(legacy, fs::read(&state_path).unwrap());
}

#[test]
fn active_and_restart_state_loss_fail_without_recreation_or_mutation() {
    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    fs::remove_file(&state_path).unwrap();
    assert_eq!(
        domain.test_append_command_attempt(1).unwrap_err(),
        StateError::PriorSessionUncertain
    );
    assert!(!state_path.exists());

    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    fs::write(&state_path, b"").unwrap();
    assert_eq!(
        domain.test_append_command_attempt(2).unwrap_err(),
        StateError::PriorSessionUncertain
    );
    assert_eq!(fs::read(&state_path).unwrap(), b"");

    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    fs::remove_file(&state_path).unwrap();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), slot).unwrap_err(),
        StateError::PriorSessionUncertain
    );
    assert!(!state_path.exists());

    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let state_path = domain.state_directory().join(STATE_FILE);
    fs::write(&state_path, b"").unwrap();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), slot).unwrap_err(),
        StateError::PriorSessionUncertain
    );
    assert_eq!(fs::read(&state_path).unwrap(), b"");
}

#[test]
fn replacing_valid_anchor_inode_breaks_persisted_domain_binding() {
    let root = tempfile::tempdir().unwrap();
    let slot = Uuid::new_v4();
    let domain = MarketStreamDomain::for_test(root.path(), slot).unwrap();
    let before = fs::read(domain.state_directory().join(STATE_FILE)).unwrap();
    let (_, anchors) = paths(root.path());
    fs::remove_file(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    fs::File::create(anchors.join(CONNECTION_LOCK_FILE)).unwrap();
    fs::set_permissions(
        anchors.join(CONNECTION_LOCK_FILE),
        Permissions::from_mode(0o440),
    )
    .unwrap();
    assert_eq!(
        MarketStreamDomain::for_test(root.path(), slot).unwrap_err(),
        StateError::InvalidState
    );
    assert_eq!(
        before,
        fs::read(domain.state_directory().join(STATE_FILE)).unwrap()
    );
}
