//! Read-only committed-manifest evidence regressions for the daily price path.

#[cfg(unix)]
mod unix_tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::Path;

    use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
    use market_data::contract::{
        FetchMode, MARKET_KR, PROVIDER_KRX, RequestMetadata, ResponseKind,
    };
    use market_data::storage::{
        CommittedManifestReadLimits, FileEntry, ManifestEntry, RawStore, StoreError,
    };
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        store: RawStore,
        entry: ManifestEntry,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        let entry = ManifestEntry {
            batch_id: BatchId::generate(),
            provider: PROVIDER_KRX.to_owned(),
            market: MARKET_KR.to_owned(),
            date: TradingDate::parse("2026-10-13").unwrap(),
            retrieved_at: UtcTimestamp::parse_rfc3339("2026-10-13T07:00:00Z").unwrap(),
            mode: FetchMode::Synthetic,
            entitlement_reference: None,
            files: Vec::new(),
        };
        store
            .append_manifest(PROVIDER_KRX, MARKET_KR, &entry)
            .unwrap();
        set_private_control_modes(&store);
        Fixture {
            _temp: temp,
            store,
            entry,
        }
    }

    fn limits(max_manifest_bytes: usize, max_records: usize) -> CommittedManifestReadLimits {
        CommittedManifestReadLimits {
            max_manifest_bytes,
            max_records,
        }
    }

    fn trusted_uid(store: &RawStore) -> u32 {
        fs::metadata(store.root().join("raw")).unwrap().uid()
    }

    fn manifest_path(store: &RawStore) -> std::path::PathBuf {
        store.manifest_path(PROVIDER_KRX, MARKET_KR)
    }

    fn lock_path(store: &RawStore) -> std::path::PathBuf {
        manifest_path(store).with_file_name("commit.lock")
    }

    fn set_private_control_modes(store: &RawStore) {
        fs::set_permissions(manifest_path(store), fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(lock_path(store), fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn read(
        store: &RawStore,
        max_manifest_bytes: usize,
        max_records: usize,
    ) -> Result<Vec<market_data::storage::CommittedManifestEvidenceRecord>, StoreError> {
        store.read_committed_manifest_evidence(
            PROVIDER_KRX,
            MARKET_KR,
            trusted_uid(store),
            limits(max_manifest_bytes, max_records),
        )
    }

    fn reordered_json(entry: &ManifestEntry) -> Vec<u8> {
        let value = serde_json::to_value(entry).unwrap();
        let keys = [
            "files",
            "entitlement_reference",
            "mode",
            "retrieved_at",
            "date",
            "market",
            "provider",
            "batch_id",
        ];
        let members = keys
            .iter()
            .map(|key| {
                format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    serde_json::to_string(&value[*key]).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{members}}}").into_bytes()
    }

    fn write_manifest(store: &RawStore, bytes: &[u8]) {
        fs::write(manifest_path(store), bytes).unwrap();
    }

    #[test]
    fn returns_exact_crlf_whitespace_and_reordered_json_line_hash() {
        let fixture = fixture();
        let mut exact_line = b" \t".to_vec();
        exact_line.extend_from_slice(&reordered_json(&fixture.entry));
        exact_line.extend_from_slice(b" \r");
        let mut file_bytes = b" \t\r\n".to_vec();
        file_bytes.extend_from_slice(&exact_line);
        file_bytes.push(b'\n');
        write_manifest(&fixture.store, &file_bytes);

        let records = read(&fixture.store, file_bytes.len(), 1).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].entry(), &fixture.entry);
        assert_eq!(records[0].line_bytes(), exact_line);
        assert_eq!(
            records[0].line_sha256(),
            &ContentHash::from_bytes(&exact_line)
        );
        assert_eq!(records[0].line_number(), 2);
        assert_eq!(fs::read(manifest_path(&fixture.store)).unwrap(), file_bytes);
    }

    #[test]
    fn accepts_complete_unterminated_record_and_ignores_torn_eof_tail() {
        let fixture = fixture();
        let complete_line = serde_json::to_vec(&fixture.entry).unwrap();
        let mut bytes = complete_line.clone();
        bytes.extend_from_slice(b"\n{\"batch_id\":");
        write_manifest(&fixture.store, &bytes);

        let records = read(&fixture.store, bytes.len(), 2).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].line_bytes(), complete_line);
        assert_eq!(
            records[0].line_sha256(),
            &ContentHash::from_bytes(&complete_line)
        );
        assert_eq!(records[0].line_number(), 1);

        write_manifest(&fixture.store, &complete_line);
        let records = read(&fixture.store, complete_line.len(), 1).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].line_bytes(), complete_line);
    }

    #[test]
    fn rejects_malformed_scope_and_filename_records_without_echoing_metadata() {
        let fixture = fixture();
        write_manifest(&fixture.store, b"not-json\n");
        assert!(matches!(
            read(&fixture.store, 64, 1),
            Err(StoreError::CorruptManifest { line: 1, .. })
        ));

        let mut wrong_scope = fixture.entry.clone();
        wrong_scope.market = "us".to_owned();
        write_manifest(&fixture.store, &serde_json::to_vec(&wrong_scope).unwrap());
        assert!(matches!(
            read(&fixture.store, 4096, 1),
            Err(StoreError::CommittedManifestRecordScopeMismatch { line: 1, .. })
        ));

        let mut invalid_name = fixture.entry.clone();
        invalid_name.files.push(FileEntry {
            kind: ResponseKind::Reference,
            file_name: "../private-metadata.json".to_owned(),
            content_hash: ContentHash::from_bytes(b"synthetic"),
            size_bytes: 9,
            request: RequestMetadata {
                endpoint: "test".to_owned(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: FetchMode::Synthetic,
            },
            response_continuation: None,
            copied_from: None,
        });
        write_manifest(&fixture.store, &serde_json::to_vec(&invalid_name).unwrap());
        let error = match read(&fixture.store, 4096, 1) {
            Ok(_) => panic!("invalid provider filename must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            &error,
            StoreError::InvalidCommittedManifestRecord { line: 1, .. }
        ));
        assert!(!error.to_string().contains("../private-metadata.json"));
    }

    #[test]
    fn returns_first_equal_duplicate_and_rejects_conflicting_metadata() {
        let fixture = fixture();
        let first = {
            let mut line = b" ".to_vec();
            line.extend_from_slice(&serde_json::to_vec(&fixture.entry).unwrap());
            line
        };
        let second = reordered_json(&fixture.entry);
        let mut bytes = first.clone();
        bytes.push(b'\n');
        bytes.extend_from_slice(&second);
        bytes.push(b'\n');
        write_manifest(&fixture.store, &bytes);

        let records = read(&fixture.store, bytes.len(), 2).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].line_bytes(), first);
        assert_eq!(records[0].line_number(), 1);

        let mut conflicting = fixture.entry.clone();
        conflicting.entitlement_reference = Some("different-record-metadata".to_owned());
        let mut conflict_bytes = serde_json::to_vec(&fixture.entry).unwrap();
        conflict_bytes.push(b'\n');
        conflict_bytes.extend_from_slice(&serde_json::to_vec(&conflicting).unwrap());
        conflict_bytes.push(b'\n');
        write_manifest(&fixture.store, &conflict_bytes);
        assert!(matches!(
            read(&fixture.store, conflict_bytes.len(), 2),
            Err(StoreError::CommittedManifestRecordConflict { line: 2, .. })
        ));
    }

    #[test]
    fn preserves_first_occurrence_order_across_unique_batches_and_duplicates() {
        let fixture = fixture();
        let mut second_entry = fixture.entry.clone();
        second_entry.batch_id = BatchId::generate();
        let first_line = serde_json::to_vec(&fixture.entry).unwrap();
        let second_line = serde_json::to_vec(&second_entry).unwrap();
        let mut bytes = first_line.clone();
        bytes.push(b'\n');
        bytes.extend_from_slice(&second_line);
        bytes.push(b'\n');
        bytes.extend_from_slice(&reordered_json(&fixture.entry));
        bytes.push(b'\n');
        write_manifest(&fixture.store, &bytes);

        let records = read(&fixture.store, bytes.len(), 3).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].entry().batch_id, fixture.entry.batch_id);
        assert_eq!(records[0].line_bytes(), first_line);
        assert_eq!(records[0].line_number(), 1);
        assert_eq!(records[1].entry().batch_id, second_entry.batch_id);
        assert_eq!(records[1].line_bytes(), second_line);
        assert_eq!(records[1].line_number(), 2);
    }

    #[test]
    fn enforces_exact_byte_and_record_bounds_before_deduplication() {
        let fixture = fixture();
        let line = serde_json::to_vec(&fixture.entry).unwrap();
        let mut one_record = line.clone();
        one_record.push(b'\n');
        write_manifest(&fixture.store, &one_record);

        assert_eq!(read(&fixture.store, one_record.len(), 1).unwrap().len(), 1);
        assert!(matches!(
            read(&fixture.store, one_record.len() - 1, 1),
            Err(StoreError::CommittedManifestByteLimitExceeded { .. })
        ));

        let mut duplicate_records = one_record.clone();
        duplicate_records.extend_from_slice(&line);
        duplicate_records.push(b'\n');
        write_manifest(&fixture.store, &duplicate_records);
        assert!(matches!(
            read(&fixture.store, duplicate_records.len(), 1),
            Err(StoreError::CommittedManifestRecordLimitExceeded { max_records: 1 })
        ));
        assert_eq!(
            read(&fixture.store, duplicate_records.len(), 2)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rejects_zero_and_overflowing_limits_before_filesystem_access() {
        let temp = tempfile::tempdir().unwrap();
        let store = RawStore::new(temp.path());
        let uid = 1234;

        assert!(matches!(
            store.read_committed_manifest_evidence(PROVIDER_KRX, MARKET_KR, uid, limits(0, 1)),
            Err(StoreError::InvalidCommittedManifestReadLimits { .. })
        ));
        assert!(matches!(
            store.read_committed_manifest_evidence(PROVIDER_KRX, MARKET_KR, uid, limits(1, 0)),
            Err(StoreError::InvalidCommittedManifestReadLimits { .. })
        ));
        assert!(matches!(
            store.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                uid,
                limits(usize::MAX, 1)
            ),
            Err(StoreError::CommittedManifestReadLimitOverflow)
        ));
        assert!(!temp.path().join("raw").exists());
    }

    #[test]
    fn missing_tree_and_missing_lock_are_not_created_or_repaired() {
        let absent = tempfile::tempdir().unwrap();
        let absent_store = RawStore::new(absent.path());
        assert!(read(&absent_store, 4096, 10).is_err());
        assert!(!absent.path().join("raw").exists());

        let fixture = fixture();
        let manifest = manifest_path(&fixture.store);
        let lock = lock_path(&fixture.store);
        let original = fs::read(&manifest).unwrap();
        fs::remove_file(&lock).unwrap();
        assert!(read(&fixture.store, 4096, 10).is_err());
        assert!(!lock.exists());
        assert_eq!(fs::read(manifest).unwrap(), original);
    }

    #[test]
    fn rejects_symlinked_raw_root_and_manifest_redirect() {
        let parent = tempfile::tempdir().unwrap();
        let actual_root = parent.path().join("actual-data");
        fs::create_dir(&actual_root).unwrap();
        let actual_store = RawStore::new(&actual_root);
        let entry = ManifestEntry {
            batch_id: BatchId::generate(),
            provider: PROVIDER_KRX.to_owned(),
            market: MARKET_KR.to_owned(),
            date: TradingDate::parse("2026-10-13").unwrap(),
            retrieved_at: UtcTimestamp::parse_rfc3339("2026-10-13T07:00:00Z").unwrap(),
            mode: FetchMode::Synthetic,
            entitlement_reference: None,
            files: Vec::new(),
        };
        actual_store
            .append_manifest(PROVIDER_KRX, MARKET_KR, &entry)
            .unwrap();
        set_private_control_modes(&actual_store);
        let alias_root = parent.path().join("data-alias");
        symlink(&actual_root, &alias_root).unwrap();
        let alias_store = RawStore::new(&alias_root);
        assert!(matches!(
            alias_store.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                trusted_uid(&actual_store),
                limits(4096, 10)
            ),
            Err(StoreError::UnsafePath { .. })
        ));

        let alias_parent = parent.path().join("parent-alias");
        symlink(parent.path(), &alias_parent).unwrap();
        let ancestor_store = RawStore::new(alias_parent.join("actual-data"));
        assert!(matches!(
            ancestor_store.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                trusted_uid(&actual_store),
                limits(4096, 10)
            ),
            Err(StoreError::UnsafePath { .. })
        ));

        let fixture = fixture();
        let path = manifest_path(&fixture.store);
        let redirected_target = path.with_file_name("manifest-real.jsonl");
        fs::rename(&path, &redirected_target).unwrap();
        symlink(&redirected_target, &path).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));

        let fixture = fixture();
        let lock = lock_path(&fixture.store);
        let redirected_lock = lock.with_file_name("commit-lock-real");
        fs::rename(&lock, &redirected_lock).unwrap();
        symlink(&redirected_lock, &lock).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));
    }

    #[test]
    fn rejects_control_directory_symlink_redirect() {
        let fixture = fixture();
        let market_dir = manifest_path(&fixture.store).parent().unwrap().to_owned();
        let saved_dir = market_dir.with_file_name("market-saved");
        fs::rename(&market_dir, &saved_dir).unwrap();
        symlink(&saved_dir, &market_dir).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));
    }

    #[test]
    fn rejects_non_regular_controls_wrong_expected_uid_and_writable_modes() {
        let fixture = fixture();
        let wrong_uid = trusted_uid(&fixture.store).wrapping_add(1);
        // This checks the explicit expected UID value only. It does not
        // simulate a Raw tree owned by another OS UID or exercise ACL rules.
        assert!(matches!(
            fixture.store.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                wrong_uid,
                limits(4096, 10)
            ),
            Err(StoreError::UnsafePath { .. })
        ));

        let manifest = manifest_path(&fixture.store);
        let control_dir = manifest.parent().unwrap();
        let dir_mode = fs::symlink_metadata(control_dir)
            .unwrap()
            .permissions()
            .mode();
        fs::set_permissions(control_dir, fs::Permissions::from_mode(dir_mode | 0o020)).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));
        fs::set_permissions(control_dir, fs::Permissions::from_mode(dir_mode)).unwrap();

        let metadata = fs::symlink_metadata(&manifest).unwrap();
        fs::set_permissions(
            &manifest,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o020),
        )
        .unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));

        let lock = lock_path(&fixture.store);
        fs::set_permissions(&manifest, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));

        let fixture = fixture();
        let manifest = manifest_path(&fixture.store);
        fs::remove_file(&manifest).unwrap();
        fs::create_dir(&manifest).unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));

        let fixture = fixture();
        let raw_root = fixture.store.root().join("raw");
        let metadata = fs::symlink_metadata(&raw_root).unwrap();
        fs::set_permissions(
            &raw_root,
            fs::Permissions::from_mode(metadata.permissions().mode() | 0o002),
        )
        .unwrap();
        assert!(matches!(
            read(&fixture.store, 4096, 10),
            Err(StoreError::UnsafePath { .. })
        ));
    }

    #[test]
    fn valid_read_is_byte_for_byte_read_only() {
        let fixture = fixture();
        let manifest = manifest_path(&fixture.store);
        let lock = lock_path(&fixture.store);
        let before_manifest = fs::read(&manifest).unwrap();
        let before_lock_len = fs::metadata(&lock).unwrap().len();

        let records = read(&fixture.store, 4096, 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(fs::read(&manifest).unwrap(), before_manifest);
        assert_eq!(fs::metadata(&lock).unwrap().len(), before_lock_len);
    }

    #[test]
    fn outside_raw_data_root_mode_is_not_treated_as_raw_owner_policy() {
        let fixture = fixture();
        let data_root = fixture.store.root();
        let mode = fs::symlink_metadata(data_root)
            .unwrap()
            .permissions()
            .mode();
        fs::set_permissions(data_root, fs::Permissions::from_mode(mode | 0o022)).unwrap();
        assert_eq!(read(&fixture.store, 4096, 10).unwrap().len(), 1);
    }

    #[test]
    fn rejects_relative_and_parent_component_roots() {
        let temp = tempfile::tempdir().unwrap();
        let relative = RawStore::new(Path::new("relative/data"));
        assert!(matches!(
            relative.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                1234,
                limits(4096, 10)
            ),
            Err(StoreError::UnsafePath { .. })
        ));

        let parent_component = RawStore::new(temp.path().join("..").join("outside-data"));
        assert!(matches!(
            parent_component.read_committed_manifest_evidence(
                PROVIDER_KRX,
                MARKET_KR,
                1234,
                limits(4096, 10)
            ),
            Err(StoreError::UnsafePath { .. })
        ));
        assert!(!temp.path().join("raw").exists());
    }
}

#[cfg(not(unix))]
#[test]
fn fails_closed_without_unix_descriptor_relative_opens() {
    let store = market_data::storage::RawStore::new("/data/raw-test");
    assert!(matches!(
        store.read_committed_manifest_evidence(
            "krx",
            "kr",
            0,
            market_data::storage::CommittedManifestReadLimits {
                max_manifest_bytes: 4096,
                max_records: 10,
            }
        ),
        Err(market_data::storage::StoreError::UnsafePath { .. })
    ));
}
