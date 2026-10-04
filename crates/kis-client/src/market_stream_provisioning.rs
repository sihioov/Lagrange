//! Fixed-path, one-shot provisioning for the production market-stream state.
//!
//! This module is available only with `market-stream-provisioning`. Its
//! public surface accepts a credential slot and generation; filesystem paths,
//! owner IDs, serialization, and fixture capabilities remain private.
//!
//! The internal layout capability is not part of the library interface:
//!
//! ```compile_fail
//! use kis_client::market_stream_state::initialize_provisioning_at;
//! ```
//!
//! ```compile_fail
//! use kis_client::market_stream_state::ProvisioningOwners;
//! ```

#[cfg(test)]
use std::fs::File;

use uuid::Uuid;

use crate::market_stream_state::{
    ProvisioningOwners, StateError, initialize_provisioning_at,
    open_production_provisioning_parent, validate_provisioning_at,
};

const RUNTIME_UID: u32 = 10_001;
const RUNTIME_GID: u32 = 10_001;

unsafe extern "C" {
    fn getuid() -> u32;
    fn geteuid() -> u32;
    fn getgid() -> u32;
    fn getegid() -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActorIds {
    uid: u32,
    euid: u32,
    gid: u32,
    egid: u32,
}

impl ActorIds {
    fn current() -> Self {
        // SAFETY: these process identity calls have no preconditions or side effects.
        unsafe {
            Self {
                uid: getuid(),
                euid: geteuid(),
                gid: getgid(),
                egid: getegid(),
            }
        }
    }

    fn may_initialize(self) -> bool {
        self.uid == 0 && self.euid == 0
    }

    fn may_validate(self) -> bool {
        self.may_initialize()
            || (self.uid == RUNTIME_UID
                && self.euid == RUNTIME_UID
                && self.gid == RUNTIME_GID
                && self.egid == RUNTIME_GID)
    }
}

/// The only successful outcome of first production initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionOutcome {
    Initialized,
}

/// Creates both fixed production leaves and installs a fresh, empty state.
///
/// The caller must run with both real and effective UID zero. A failure after
/// creation begins is uncertain and leaves its filesystem evidence intact.
pub fn initialize_production_market_stream_domain(
    slot: Uuid,
    generation: u64,
) -> Result<ProvisionOutcome, StateError> {
    validate_input(slot, generation)?;
    if !ActorIds::current().may_initialize() {
        return Err(StateError::ProvisioningActorDenied);
    }
    let parent = open_production_provisioning_parent()?;
    initialize_provisioning_at(&parent, slot, generation, ProvisioningOwners::production())?;
    Ok(ProvisionOutcome::Initialized)
}

/// Validates the fixed production layout without changing its contents.
///
/// Root may validate; the runtime identity is exactly UID:GID 10001:10001.
pub fn validate_production_market_stream_domain(
    slot: Uuid,
    generation: u64,
) -> Result<(), StateError> {
    validate_input(slot, generation)?;
    if !ActorIds::current().may_validate() {
        return Err(StateError::ProvisioningActorDenied);
    }
    let parent = open_production_provisioning_parent()?;
    validate_provisioning_at(&parent, slot, generation, ProvisioningOwners::production())
}

fn validate_input(slot: Uuid, generation: u64) -> Result<(), StateError> {
    if slot.is_nil() || generation == 0 {
        return Err(StateError::ProvisioningInputInvalid);
    }
    Ok(())
}

#[cfg(all(test, feature = "market-stream-provisioning"))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    use super::*;
    use crate::market_stream_state::{
        CONNECTION_LOCK_FILE, ProvisioningSyncPoint, STATE_FILE, STATE_LOCK_FILE,
        fail_provisioning_fsync_at, initialize_provisioning_at, open_fixture_provisioning_parent,
        race_provisioning_install, validate_provisioning_at,
    };

    const GENERATION: u64 = 17;

    struct Fixture {
        root: tempfile::TempDir,
        parent: File,
        owners: ProvisioningOwners,
        slot: Uuid,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let metadata = fs::metadata(root.path()).unwrap();
            let owners = ProvisioningOwners::fixture(metadata.uid(), metadata.gid());
            let parent =
                open_fixture_provisioning_parent(root.path(), metadata.uid(), metadata.gid())
                    .unwrap();
            Self {
                root,
                parent,
                owners,
                slot: Uuid::new_v4(),
            }
        }

        fn initialize(&self) -> Result<(), StateError> {
            initialize_provisioning_at(&self.parent, self.slot, GENERATION, self.owners)
        }

        fn validate(&self) -> Result<(), StateError> {
            validate_provisioning_at(&self.parent, self.slot, GENERATION, self.owners)
        }

        fn state_dir(&self) -> std::path::PathBuf {
            self.root.path().join("kis-market-stream")
        }

        fn anchors_dir(&self) -> std::path::PathBuf {
            self.root.path().join("kis-market-stream-locks")
        }
    }

    fn identity(path: &Path) -> (u64, u64) {
        let metadata = fs::metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    fn assert_file_metadata(path: &Path, uid: u32, gid: u32, mode: u32, size: u64) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.gid(), gid);
        assert_eq!(metadata.permissions().mode() & 0o7777, mode);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.len(), size);
    }

    #[test]
    fn actor_rules_require_root_initialization_and_root_or_exact_runtime_validation() {
        assert!(
            ActorIds {
                uid: 0,
                euid: 0,
                gid: 0,
                egid: 0
            }
            .may_initialize()
        );
        assert!(
            !ActorIds {
                uid: 1000,
                euid: 0,
                gid: 0,
                egid: 0
            }
            .may_initialize()
        );
        assert!(
            !ActorIds {
                uid: 0,
                euid: 1000,
                gid: 0,
                egid: 0
            }
            .may_initialize()
        );
        assert!(
            ActorIds {
                uid: 0,
                euid: 0,
                gid: 20,
                egid: 20
            }
            .may_validate()
        );
        assert!(
            ActorIds {
                uid: RUNTIME_UID,
                euid: RUNTIME_UID,
                gid: RUNTIME_GID,
                egid: RUNTIME_GID,
            }
            .may_validate()
        );
        assert!(
            !ActorIds {
                uid: RUNTIME_UID,
                euid: RUNTIME_UID,
                gid: RUNTIME_GID,
                egid: 1000,
            }
            .may_validate()
        );
        assert!(
            !ActorIds {
                uid: 1000,
                euid: 1000,
                gid: 1000,
                egid: 1000
            }
            .may_validate()
        );
    }

    #[test]
    fn trusted_fixture_parent_rejects_group_writable_and_symlink_parents() {
        let writable = tempfile::tempdir().unwrap();
        fs::set_permissions(writable.path(), fs::Permissions::from_mode(0o770)).unwrap();
        let writable_meta = fs::metadata(writable.path()).unwrap();
        assert_eq!(
            open_fixture_provisioning_parent(
                writable.path(),
                writable_meta.uid(),
                writable_meta.gid(),
            )
            .unwrap_err(),
            StateError::UnsafePath
        );

        let target = tempfile::tempdir().unwrap();
        fs::set_permissions(target.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let alias_root = tempfile::tempdir().unwrap();
        fs::set_permissions(alias_root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let alias = alias_root.path().join("parent-link");
        std::os::unix::fs::symlink(target.path(), &alias).unwrap();
        let target_meta = fs::metadata(target.path()).unwrap();
        assert_eq!(
            open_fixture_provisioning_parent(&alias, target_meta.uid(), target_meta.gid())
                .unwrap_err(),
            StateError::UnsafePath
        );
    }

    #[test]
    fn fresh_install_uses_exact_layout_empty_schema_and_two_distinct_anchors() {
        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fixture.validate().unwrap();
        let state_meta = fs::metadata(fixture.state_dir()).unwrap();
        let anchor_meta = fs::metadata(fixture.anchors_dir()).unwrap();
        let parent_meta = fs::metadata(fixture.root.path()).unwrap();
        assert_eq!(state_meta.uid(), parent_meta.uid());
        assert_eq!(state_meta.gid(), parent_meta.gid());
        assert_eq!(state_meta.permissions().mode() & 0o7777, 0o700);
        assert_eq!(anchor_meta.uid(), parent_meta.uid());
        assert_eq!(anchor_meta.gid(), parent_meta.gid());
        assert_eq!(anchor_meta.permissions().mode() & 0o7777, 0o750);

        let connection = fixture.anchors_dir().join(CONNECTION_LOCK_FILE);
        let state_lock = fixture.anchors_dir().join(STATE_LOCK_FILE);
        assert_file_metadata(&connection, parent_meta.uid(), parent_meta.gid(), 0o440, 0);
        assert_file_metadata(&state_lock, parent_meta.uid(), parent_meta.gid(), 0o440, 0);
        assert_ne!(identity(&connection), identity(&state_lock));

        let state_path = fixture.state_dir().join(STATE_FILE);
        let bytes = fs::read(&state_path).unwrap();
        assert_file_metadata(
            &state_path,
            parent_meta.uid(),
            parent_meta.gid(),
            0o600,
            bytes.len() as u64,
        );
        let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(state["schema"], "kis-market-stream-state-v1");
        assert_eq!(state["credential_slot_id"], fixture.slot.to_string());
        assert_eq!(state["credential_generation"], GENERATION.to_string());
        assert_eq!(state["approval_key"], serde_json::Value::Null);
        assert_eq!(state["current_epoch"], serde_json::Value::Null);
        assert_eq!(state["pending_command"], serde_json::Value::Null);
        assert_eq!(state["approval_reservations_ms"], serde_json::json!([]));
        assert_eq!(state["command_attempts_ms"], serde_json::json!([]));
        assert_eq!(state["reconnect_attempts_10m_ms"], serde_json::json!([]));
        assert_eq!(state["reconnect_attempts_day_ms"], serde_json::json!([]));
        assert_eq!(state["has_connected"], false);
    }

    #[test]
    fn revalidation_preserves_bytes_histories_budgets_and_all_inodes() {
        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        let state_path = fixture.state_dir().join(STATE_FILE);
        let mut state: serde_json::Value =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        state["approval_reservations_ms"] = serde_json::json!([12]);
        state["approval_ambiguous"] = serde_json::json!(true);
        state["command_attempts_ms"] = serde_json::json!([31, 32]);
        state["reconnect_attempts_10m_ms"] = serde_json::json!([41]);
        state["reconnect_attempts_day_ms"] = serde_json::json!([42, 43]);
        fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();

        let bytes_before = fs::read(&state_path).unwrap();
        let identities_before = [
            identity(&fixture.state_dir()),
            identity(&fixture.anchors_dir()),
            identity(&fixture.anchors_dir().join(CONNECTION_LOCK_FILE)),
            identity(&fixture.anchors_dir().join(STATE_LOCK_FILE)),
            identity(&state_path),
        ];
        fixture.validate().unwrap();
        fixture.validate().unwrap();
        assert_eq!(bytes_before, fs::read(&state_path).unwrap());
        assert_eq!(
            identities_before,
            [
                identity(&fixture.state_dir()),
                identity(&fixture.anchors_dir()),
                identity(&fixture.anchors_dir().join(CONNECTION_LOCK_FILE)),
                identity(&fixture.anchors_dir().join(STATE_LOCK_FILE)),
                identity(&state_path),
            ]
        );
        let after: serde_json::Value = serde_json::from_slice(&bytes_before).unwrap();
        assert_eq!(after["approval_reservations_ms"], serde_json::json!([12]));
        assert_eq!(after["approval_ambiguous"], true);
        assert_eq!(after["command_attempts_ms"], serde_json::json!([31, 32]));
        assert_eq!(after["reconnect_attempts_10m_ms"], serde_json::json!([41]));
        assert_eq!(
            after["reconnect_attempts_day_ms"],
            serde_json::json!([42, 43])
        );
    }

    #[test]
    fn inputs_generation_and_domain_must_match_without_relabeling() {
        let fixture = Fixture::new();
        assert_eq!(
            initialize_provisioning_at(&fixture.parent, Uuid::nil(), GENERATION, fixture.owners),
            Err(StateError::ProvisioningInputInvalid)
        );
        assert_eq!(
            initialize_provisioning_at(&fixture.parent, fixture.slot, 0, fixture.owners),
            Err(StateError::ProvisioningInputInvalid)
        );
        fixture.initialize().unwrap();
        assert_eq!(
            validate_provisioning_at(
                &fixture.parent,
                fixture.slot,
                GENERATION + 1,
                fixture.owners
            ),
            Err(StateError::InvalidState)
        );
        assert_eq!(
            validate_provisioning_at(&fixture.parent, Uuid::new_v4(), GENERATION, fixture.owners,),
            Err(StateError::InvalidState)
        );
        assert_eq!(fixture.initialize(), Err(StateError::PriorSessionUncertain));
    }

    #[test]
    fn partial_and_symlink_layouts_fail_without_repair_or_cleanup() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.state_dir()).unwrap();
        fs::set_permissions(&fixture.state_dir(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fixture.initialize(), Err(StateError::PriorSessionUncertain));
        assert!(!fixture.anchors_dir().exists());
        assert!(!fixture.state_dir().join(STATE_FILE).exists());

        let fixture = Fixture::new();
        let target = fixture.root.path().join("anchors-target");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, fixture.anchors_dir()).unwrap();
        assert_eq!(fixture.initialize(), Err(StateError::PriorSessionUncertain));
        assert!(
            fs::symlink_metadata(fixture.anchors_dir())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!fixture.state_dir().exists());
    }

    #[test]
    fn missing_zero_corrupt_symlink_and_hardlinked_state_fail_closed() {
        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::remove_file(fixture.state_dir().join(STATE_FILE)).unwrap();
        assert!(fixture.validate().is_err());

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::write(fixture.state_dir().join(STATE_FILE), b"").unwrap();
        assert_eq!(fixture.validate(), Err(StateError::PriorSessionUncertain));

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::write(fixture.state_dir().join(STATE_FILE), b"not-json").unwrap();
        assert_eq!(fixture.validate(), Err(StateError::InvalidState));

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        let state = fixture.state_dir().join(STATE_FILE);
        let moved = fixture.root.path().join("state-target");
        fs::rename(&state, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &state).unwrap();
        assert_eq!(fixture.validate(), Err(StateError::UnsafePath));

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::hard_link(
            fixture.state_dir().join(STATE_FILE),
            fixture.root.path().join("state-hardlink"),
        )
        .unwrap();
        assert_eq!(fixture.validate(), Err(StateError::UnsafePath));
    }

    #[test]
    fn anchor_and_directory_inode_replacements_fail_closed() {
        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        let anchor = fixture.anchors_dir().join(CONNECTION_LOCK_FILE);
        let original = identity(&anchor);
        fs::remove_file(&anchor).unwrap();
        let replacement = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o440)
            .open(&anchor)
            .unwrap();
        replacement.sync_all().unwrap();
        assert_ne!(identity(&anchor), original);
        assert!(fixture.validate().is_err());

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        let state_path = fixture.state_dir();
        let moved = fixture.root.path().join("old-state-dir");
        fs::rename(&state_path, &moved).unwrap();
        fs::create_dir(&state_path).unwrap();
        fs::set_permissions(&state_path, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(fixture.validate().is_err());
    }

    #[test]
    fn hardlinked_anchor_is_rejected_without_mutation() {
        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        let connection = fixture.anchors_dir().join(CONNECTION_LOCK_FILE);
        let target = fixture.root.path().join("anchor-symlink-target");
        fs::write(&target, b"").unwrap();
        fs::remove_file(&connection).unwrap();
        std::os::unix::fs::symlink(&target, &connection).unwrap();
        assert_eq!(fixture.validate(), Err(StateError::UnsafePath));

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::hard_link(
            fixture.anchors_dir().join(CONNECTION_LOCK_FILE),
            fixture.root.path().join("anchor-hardlink"),
        )
        .unwrap();
        assert_eq!(fixture.validate(), Err(StateError::UnsafePath));

        let fixture = Fixture::new();
        fixture.initialize().unwrap();
        fs::remove_file(fixture.anchors_dir().join(STATE_LOCK_FILE)).unwrap();
        assert_eq!(fixture.validate(), Err(StateError::UnsafePath));
    }

    #[test]
    fn concurrent_fresh_initializers_have_one_winner_and_one_uncertain_observer() {
        let fixture = Fixture::new();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let parent = fixture.parent.try_clone().unwrap();
            let owners = fixture.owners;
            let slot = fixture.slot;
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                initialize_provisioning_at(&parent, slot, GENERATION, owners)
            }));
        }
        barrier.wait();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        fixture.validate().unwrap();
    }

    #[test]
    fn fsync_and_no_replace_failures_preserve_partial_evidence() {
        let fixture = Fixture::new();
        fail_provisioning_fsync_at(Some(ProvisioningSyncPoint::AnchorDirectoryAfterAnchors));
        let result = fixture.initialize();
        fail_provisioning_fsync_at(None);
        assert_eq!(result, Err(StateError::ProvisioningUncertain));
        assert!(fixture.state_dir().is_dir());
        assert!(fixture.anchors_dir().join(CONNECTION_LOCK_FILE).exists());
        assert!(fixture.anchors_dir().join(STATE_LOCK_FILE).exists());
        assert!(!fixture.state_dir().join(STATE_FILE).exists());
        assert!(fixture.validate().is_err());
        assert_eq!(fixture.initialize(), Err(StateError::PriorSessionUncertain));

        let fixture = Fixture::new();
        fail_provisioning_fsync_at(Some(ProvisioningSyncPoint::StateTemporaryFile));
        let result = fixture.initialize();
        fail_provisioning_fsync_at(None);
        assert_eq!(result, Err(StateError::ProvisioningUncertain));
        assert!(fixture.state_dir().join(STATE_FILE).exists() == false);
        let temporary = fs::read_dir(fixture.state_dir())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            temporary
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".provision.tmp")
        );
        assert!(fixture.validate().is_err());
        assert_eq!(fixture.initialize(), Err(StateError::PriorSessionUncertain));

        let fixture = Fixture::new();
        race_provisioning_install(true);
        let result = fixture.initialize();
        race_provisioning_install(false);
        assert_eq!(result, Err(StateError::ProvisioningUncertain));
        assert_eq!(
            fs::read(fixture.state_dir().join(STATE_FILE)).unwrap(),
            b"race"
        );
        assert!(fs::read_dir(fixture.state_dir()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".provision.tmp")
        }));
    }
}
