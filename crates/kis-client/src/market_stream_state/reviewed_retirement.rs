//! Operator-only retirement of a proven dead, idle owned transport.
//! The host reviewer must stop and drain the exact runner first. This is not
//! a runtime retry: ambiguous commands, handshakes and approval issuance stay
//! fenced. An immutable intent precedes the only permitted state transition.

use super::*;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_exact(store: &WsStateStore) -> Result<(Vec<u8>, ObjectIdentity), StateError> {
    let file = open_at(&store.directory, STATE_FILE, O_RDONLY, 0)
        .map_err(|_| StateError::PriorSessionUncertain)?;
    validate_regular_descriptor(
        &file,
        store.state_uid,
        store.state_gid,
        STATE_FILE_MODE,
        true,
    )?;
    let metadata = file.metadata().map_err(|_| StateError::Io)?;
    if metadata.len() == 0 || metadata.len() > MAX_STATE_FILE_BYTES {
        return Err(StateError::InvalidState);
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| StateError::Io)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(StateError::PriorSessionUncertain);
    }
    Ok((bytes, ObjectIdentity::from_metadata(&metadata)))
}

fn create_retirement_file(
    store: &WsStateStore,
    name: &str,
    bytes: &[u8],
    point: ProvisioningSyncPoint,
) -> Result<(), StateError> {
    let mut file = open_at(
        &store.directory,
        name,
        O_WRONLY | O_CREAT | O_EXCL,
        STATE_FILE_MODE,
    )
    .map_err(|_| StateError::ProvisioningUncertain)?;
    // No failed or partial record is removed, overwritten, or retried.
    set_new_object_owner_mode(&file, store.state_uid, store.state_gid, STATE_FILE_MODE)?;
    file.write_all(bytes).map_err(|_| StateError::Io)?;
    provisioning_sync(&file, point)?;
    store.directory.sync_all().map_err(|_| StateError::Io)
}

pub(crate) fn retire_reviewed_epoch_at(
    parent: &File,
    slot: Uuid,
    generation: u64,
    expected_state_sha256: &str,
    host_review_sha256: &str,
    owners: ProvisioningOwners,
) -> Result<(), StateError> {
    let valid_hash = |s: &str| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            && s != "0".repeat(64)
    };
    if !valid_hash(expected_state_sha256) || !valid_hash(host_review_sha256) {
        return Err(StateError::ProvisioningInputInvalid);
    }
    let store = provisioning_store_at(parent, slot, owners)?;
    let _connection = store.lock()?;
    let _state = store.state_guard()?;
    let (original, identity) = read_exact(&store)?;
    if digest(&original) != expected_state_sha256 {
        return Err(StateError::PriorSessionUncertain);
    }
    let mut state = store.load_unlocked()?;
    // Reject unknown fields/noncanonical bytes rather than dropping later evidence.
    if serde_json::to_vec(&state).map_err(|_| StateError::Serialization)? != original
        || state.credential_generation != generation.to_string()
        || !state.has_connected
        || state.current_epoch.is_none()
        || state.pending_command.is_some()
        || state.connection_attempt_id.is_some()
        || state.connection_attempt_started_ms.is_some()
        || state.approval_ambiguous
        || state.approval_dispatch_in_flight
    {
        return Err(StateError::PriorSessionUncertain);
    }
    let epoch = state
        .current_epoch
        .take()
        .ok_or(StateError::PriorSessionUncertain)?;
    let replacement = serde_json::to_vec(&state).map_err(|_| StateError::Serialization)?;
    let receipt = serde_json::to_vec(&serde_json::json!({
        "schema": "kis-reviewed-owned-epoch-retirement-v1", "retiredEpoch": epoch,
        "previousStateSha256": expected_state_sha256, "nextStateSha256": digest(&replacement),
        "hostReviewSha256": host_review_sha256, "transition": "current_epoch_to_none_only",
        "brokerCleanCloseAsserted": false, "providerCalls": 0
    }))
    .map_err(|_| StateError::Serialization)?;
    create_retirement_file(
        &store,
        &format!("retired-epoch-{epoch}.json"),
        &receipt,
        ProvisioningSyncPoint::RetirementIntent,
    )?;
    let temporary = format!(".retired-epoch-{epoch}.state.tmp");
    create_retirement_file(
        &store,
        &temporary,
        &replacement,
        ProvisioningSyncPoint::RetirementState,
    )?;
    store.validate_directories()?;
    if read_exact(&store)? != (original, identity) {
        return Err(StateError::PriorSessionUncertain);
    }
    rename_at(&store.directory, &temporary, STATE_FILE).map_err(|_| StateError::Io)?;
    provisioning_sync(&store.directory, ProvisioningSyncPoint::RetirementInstalled)?;
    if read_exact(&store)?.0 != replacement {
        return Err(StateError::ProvisioningUncertain);
    }
    store.load_unlocked().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Fixture {
        root: tempfile::TempDir,
        parent: File,
        slot: Uuid,
        owners: ProvisioningOwners,
        store: WsStateStore,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let m = root.path().metadata().unwrap();
            let owners = ProvisioningOwners::fixture(m.uid(), m.gid());
            let parent = open_fixture_provisioning_parent(root.path(), m.uid(), m.gid()).unwrap();
            let slot = Uuid::new_v4();
            initialize_provisioning_at(&parent, slot, 17, owners).unwrap();
            let store = provisioning_store_at(&parent, slot, owners).unwrap();
            store
                .with_locked_state(|s| {
                    s.current_epoch = Some(Uuid::new_v4().to_string());
                    s.has_connected = true;
                    s.approval_key = Some("SENTINEL_DO_NOT_COPY".into());
                    s.approval_issued_at_ms = Some(1000);
                    s.approval_expires_at_ms = Some(86401000);
                    s.approval_reservations_ms = vec![1000];
                    s.command_attempts_ms = vec![2000];
                    s.reconnect_attempts_10m_ms = vec![1500];
                    s.reconnect_attempts_day_ms = vec![1500];
                    s.next_command_ordinal = 2;
                    s.reconnect_sequence = 1;
                    s.reconnect_next_allowed_at_ms = Some(2500);
                    Ok(())
                })
                .unwrap();
            Self {
                root,
                parent,
                slot,
                owners,
                store,
            }
        }
        fn bytes(&self) -> Vec<u8> {
            read_exact(&self.store).unwrap().0
        }
        fn run(&self, hash: &str) -> Result<(), StateError> {
            retire_reviewed_epoch_at(
                &self.parent,
                self.slot,
                17,
                hash,
                &"a".repeat(64),
                self.owners,
            )
        }
    }
    #[test]
    fn retirement_preserves_every_other_field_and_anchor_and_records_original_epoch() {
        let f = Fixture::new();
        let before = f.bytes();
        let mut expected: serde_json::Value = serde_json::from_slice(&before).unwrap();
        let epoch = expected["current_epoch"].as_str().unwrap().to_owned();
        expected["current_epoch"] = serde_json::Value::Null;
        let binding = f.store.binding.clone();
        f.run(&digest(&before)).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&f.bytes()).unwrap(),
            expected
        );
        f.store.validate_directories().unwrap();
        assert_eq!(f.store.load().unwrap().domain_binding, binding);
        let receipt = fs::read(
            f.root
                .path()
                .join(PRODUCTION_STATE_LEAF)
                .join(format!("retired-epoch-{epoch}.json")),
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&receipt).contains("SENTINEL_DO_NOT_COPY"));
        let receipt: serde_json::Value = serde_json::from_slice(&receipt).unwrap();
        assert_eq!(receipt["retiredEpoch"], epoch);
        assert_eq!(receipt["previousStateSha256"], digest(&before));
        assert_eq!(receipt["nextStateSha256"], digest(&f.bytes()));
        assert!(f.run(&digest(&before)).is_err());
        assert!(f.run(&digest(&f.bytes())).is_err());
    }
    #[test]
    fn retirement_refuses_hash_mismatch_or_an_active_connection_anchor() {
        let f = Fixture::new();
        let before = f.bytes();
        assert!(f.run(&"b".repeat(64)).is_err());
        let _active = f.store.lock().unwrap();
        assert!(matches!(f.run(&digest(&before)), Err(StateError::LockBusy)));
        assert_eq!(before, f.bytes());
    }
    #[test]
    fn retirement_refuses_ambiguous_command_handshake_or_approval() {
        for kind in 0..5 {
            let f = Fixture::new();
            f.store
                .with_locked_state(|s| {
                    match kind {
                        0 => s.approval_ambiguous = true,
                        1 => s.approval_dispatch_in_flight = true,
                        2 => {
                            s.connection_attempt_id = Some(Uuid::new_v4().to_string());
                            s.connection_attempt_started_ms = Some(1000);
                        }
                        3 => {
                            s.pending_command = Some(DurablePendingCommand {
                                epoch: s.current_epoch.clone().unwrap(),
                                operation: "subscribe".into(),
                                symbol: "005930".into(),
                                ordinal: 1,
                                sent_at_ms: 1000,
                                deadline_ms: 2000,
                            })
                        }
                        _ => s.credential_generation = "18".into(),
                    }
                    Ok(())
                })
                .unwrap();
            let before = f.bytes();
            assert!(f.run(&digest(&before)).is_err());
            assert_eq!(f.bytes(), before);
        }
    }
    #[test]
    fn retirement_sync_failure_preserves_intent_and_blocks_replay() {
        for point in [
            ProvisioningSyncPoint::RetirementIntent,
            ProvisioningSyncPoint::RetirementState,
            ProvisioningSyncPoint::RetirementInstalled,
        ] {
            let f = Fixture::new();
            let before = f.bytes();
            fail_provisioning_fsync_at(Some(point));
            let result = f.run(&digest(&before));
            fail_provisioning_fsync_at(None);
            assert!(result.is_err());
            if point != ProvisioningSyncPoint::RetirementInstalled {
                assert_eq!(f.bytes(), before);
            }
            assert!(f.run(&digest(&before)).is_err());
            assert!(f.run(&digest(&f.bytes())).is_err());
        }
    }

    #[test]
    fn retirement_preserves_unknown_schema_fields_instead_of_reserializing_them_away() {
        let f = Fixture::new();
        let mut value: serde_json::Value = serde_json::from_slice(&f.bytes()).unwrap();
        value["future_evidence"] = serde_json::json!({"must_survive": true});
        let before = serde_json::to_vec(&value).unwrap();
        fs::write(
            f.root.path().join(PRODUCTION_STATE_LEAF).join(STATE_FILE),
            &before,
        )
        .unwrap();
        assert!(f.run(&digest(&before)).is_err());
        assert_eq!(f.bytes(), before);
    }
}
