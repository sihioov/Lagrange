//! S2-T01–32 regressions for the fixed-contract committed Raw snapshot.
//!
//! Guard-to-test map: S2-T01–21 and T27/T29/T30/T32 are public integration
//! cases below; T22/T23/T26 cover lock lifetime and cooperating writers here.
//! Source-module supplements use private deterministic seams: T03 duplicate
//! field/sequence parity, T04 duplicate-Arc charging, T06 stat-size conversion,
//! T07 bounded reads, T08 pre-materialization file limits, T09/T10 resource
//! observations, T13 Linux
//! root copy ordering, R15 recorded-size preflight, T23 lock replacement/identity,
//! T24 final-name mutations, T25 revalidation, and T28 cleanup order. T13
//! symlink levels are in the integration test below. T31 is target-specific.

#[cfg(target_os = "linux")]
mod linux_tests {
    use std::error::Error;
    use std::fs::{self, OpenOptions, hard_link};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;

    use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
    use fs2::FileExt;
    use market_data::contract::{
        FetchMode, MARKET_KR, RequestMetadata, ResponseKind, SourceFileReference,
    };
    use market_data::daily_price_only::source::{
        ApprovedRawScope, CommittedRawReader, DailyEvidenceError, ScanLimitValues, ScanLimits,
        SnapshotStats,
    };
    use market_data::storage::{CommittedManifestReadLimits, FileEntry, ManifestEntry, RawStore};
    use tempfile::TempDir;
    use uuid::Uuid;

    const DEFAULT_BODY: &[u8] = b"opaque provider bytes";

    struct TestBatch {
        entry: ManifestEntry,
        bodies: Vec<(String, Vec<u8>)>,
    }

    struct Fixture {
        _temp: TempDir,
        data_root: PathBuf,
        uid: u32,
        batches: Vec<TestBatch>,
    }

    fn fixture() -> Fixture {
        fixture_with(vec![
            batch(
                1,
                "2026-10-12",
                &[("bars.json", DEFAULT_BODY), ("calendar.json", b"not JSON")],
            ),
            batch(2, "2026-10-13", &[("actions.json", b"opaque action bytes")]),
        ])
    }

    fn one_batch() -> Fixture {
        fixture_with(vec![batch(1, "2026-10-13", &[("bars.json", DEFAULT_BODY)])])
    }

    fn empty_fixture() -> Fixture {
        fixture_with(Vec::new())
    }

    fn batch(id: u128, date: &str, bodies: &[(&str, &[u8])]) -> TestBatch {
        let body_entries: Vec<FileEntry> = bodies
            .iter()
            .map(|(name, bytes)| FileEntry {
                kind: ResponseKind::Bars,
                file_name: (*name).to_owned(),
                content_hash: ContentHash::from_bytes(bytes),
                size_bytes: u64::try_from(bytes.len()).unwrap(),
                request: RequestMetadata {
                    endpoint: "kis.eod.fixture".to_owned(),
                    query: vec![("date".to_owned(), date.to_owned())],
                    headers: Vec::new(),
                    mode: FetchMode::Synthetic,
                },
                response_continuation: None,
                copied_from: None,
            })
            .collect();
        TestBatch {
            entry: ManifestEntry {
                batch_id: BatchId::from_uuid(Uuid::from_u128(id)),
                provider: "kis".to_owned(),
                market: MARKET_KR.to_owned(),
                date: TradingDate::parse(date).unwrap(),
                retrieved_at: UtcTimestamp::parse_rfc3339("2026-10-14T01:02:03Z").unwrap(),
                mode: FetchMode::Synthetic,
                entitlement_reference: None,
                files: body_entries,
            },
            bodies: bodies
                .iter()
                .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
                .collect(),
        }
    }

    fn fixture_with(batches: Vec<TestBatch>) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir(&data_root).unwrap();
        set_mode(&data_root, 0o777);
        let raw = raw_root(&data_root);
        private_tree_dir(&data_root, &raw);
        let manifest_dir = manifest_dir(&data_root);
        private_tree_dir(&data_root, &manifest_dir);
        private_file(&lock_path(&data_root), b"");
        for batch in &batches {
            write_batch(&data_root, batch);
        }
        let fixture = Fixture {
            uid: fs::metadata(&raw).unwrap().uid(),
            _temp: temp,
            data_root,
            batches,
        };
        rewrite_manifest(&fixture, &manifest_rows(&fixture));
        fixture
    }

    fn raw_root(data_root: &Path) -> PathBuf {
        data_root.join("raw")
    }

    fn manifest_dir(data_root: &Path) -> PathBuf {
        raw_root(data_root)
            .join("manifests")
            .join("provider=kis")
            .join("market=kr")
    }

    fn manifest_path(data_root: &Path) -> PathBuf {
        manifest_dir(data_root).join("manifest.jsonl")
    }

    fn lock_path(data_root: &Path) -> PathBuf {
        manifest_dir(data_root).join("commit.lock")
    }

    fn batch_dir(data_root: &Path, entry: &ManifestEntry) -> PathBuf {
        raw_root(data_root)
            .join("provider=kis")
            .join("market=kr")
            .join(format!("date={}", entry.date))
            .join(format!("batch={}", entry.batch_id))
    }

    fn private_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
        set_mode(path, 0o700);
    }

    fn private_tree_dir(root: &Path, path: &Path) {
        let relative = path.strip_prefix(root).unwrap();
        let mut current = root.to_path_buf();
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                panic!("Raw fixture path must stay below its data root");
            };
            current.push(name);
            match fs::create_dir(&current) {
                Ok(()) => set_mode(&current, 0o700),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    assert!(fs::symlink_metadata(&current).unwrap().is_dir());
                }
                Err(error) => panic!("failed to create Raw fixture directory: {error}"),
            }
        }
    }

    fn private_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        set_mode(path, 0o600);
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn write_batch(data_root: &Path, batch: &TestBatch) {
        let path = batch_dir(data_root, &batch.entry);
        private_tree_dir(data_root, &path);
        let metadata = serde_json::to_vec_pretty(&batch.entry).unwrap();
        private_file(&path.join("batch.json"), &metadata);
        for (name, bytes) in &batch.bodies {
            private_file(&path.join(name), bytes);
        }
    }

    fn manifest_rows(fixture: &Fixture) -> Vec<u8> {
        let mut bytes = Vec::new();
        for batch in &fixture.batches {
            bytes.extend(serde_json::to_vec(&batch.entry).unwrap());
            bytes.push(b'\n');
        }
        bytes
    }

    fn rewrite_manifest(fixture: &Fixture, bytes: &[u8]) {
        private_file(&manifest_path(&fixture.data_root), bytes);
    }

    fn scope(fixture: &Fixture) -> ApprovedRawScope {
        ApprovedRawScope::new("kis", "kr", fixture.uid).unwrap()
    }

    fn values() -> ScanLimitValues {
        ScanLimitValues {
            max_manifest_bytes: 65_536,
            max_manifest_records: 16,
            max_batches: 8,
            max_files_per_batch: 8,
            max_files: 16,
            max_batch_metadata_bytes: 16_384,
            max_source_file_bytes: 8_192,
            max_retained_bytes: 262_144,
            max_open_descriptors: 128,
            max_data_root_bytes: 4_096,
            max_data_root_components: 32,
            max_metadata_json_depth: 32,
        }
    }

    fn limits() -> ScanLimits {
        ScanLimits::new(values()).unwrap()
    }

    fn custom_limits(change: impl FnOnce(&mut ScanLimitValues)) -> ScanLimits {
        let mut configured = values();
        change(&mut configured);
        ScanLimits::new(configured).unwrap()
    }

    fn scan(
        fixture: &Fixture,
        scan_limits: ScanLimits,
    ) -> Result<market_data::daily_price_only::source::CommittedRawSnapshot, DailyEvidenceError>
    {
        CommittedRawReader::new(&fixture.data_root).snapshot(scope(fixture), scan_limits)
    }

    fn error(fixture: &Fixture, scan_limits: ScanLimits) -> DailyEvidenceError {
        scan(fixture, scan_limits)
            .err()
            .expect("snapshot should reject fixture")
    }

    fn valid_control(fixture: &Fixture) {
        assert!(
            scan(fixture, limits()).is_ok(),
            "fixture must reach its intended guard"
        );
    }

    fn component_count(path: &Path) -> usize {
        path.components()
            .filter(|component| matches!(component, std::path::Component::Normal(_)))
            .count()
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TreeState {
        path: PathBuf,
        device: u64,
        inode: u64,
        mode: u32,
        uid: u32,
        links: u64,
        contents: Option<Vec<u8>>,
    }

    fn tree_state(root: &Path) -> Vec<TreeState> {
        fn visit(root: &Path, current: &Path, state: &mut Vec<TreeState>) {
            let metadata = fs::symlink_metadata(current).unwrap();
            let contents = if metadata.is_file() {
                Some(fs::read(current).unwrap())
            } else if metadata.file_type().is_symlink() {
                Some(
                    fs::read_link(current)
                        .unwrap()
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                )
            } else {
                None
            };
            state.push(TreeState {
                path: current.strip_prefix(root).unwrap().to_path_buf(),
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                uid: metadata.uid(),
                links: metadata.nlink(),
                contents,
            });
            if metadata.is_dir() {
                let mut children: Vec<_> = fs::read_dir(current)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect();
                children.sort();
                for child in children {
                    visit(root, &child, state);
                }
            }
        }

        let mut state = Vec::new();
        visit(root, root, &mut state);
        state.sort_by(|left, right| left.path.cmp(&right.path));
        state
    }

    fn entry_reference(batch: &ManifestEntry, file: &FileEntry) -> SourceFileReference {
        SourceFileReference {
            provider: batch.provider.clone(),
            market: batch.market.clone(),
            batch_id: batch.batch_id,
            file_name: file.file_name.clone(),
            content_hash: file.content_hash.clone(),
            retrieved_at: batch.retrieved_at,
        }
    }

    fn reversed_top_level_json(entry: &ManifestEntry) -> Vec<u8> {
        let value = serde_json::to_value(entry).unwrap();
        let object = value.as_object().unwrap();
        let order = [
            "files",
            "entitlement_reference",
            "mode",
            "retrieved_at",
            "date",
            "market",
            "provider",
            "batch_id",
        ];
        let mut out = String::from("{");
        for (index, key) in order.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            out.push_str(&serde_json::to_string(key).unwrap());
            out.push(':');
            out.push_str(&serde_json::to_string(object.get(*key).unwrap()).unwrap());
        }
        out.push('}');
        out.into_bytes()
    }

    fn fd_bound(fixture: &Fixture) -> usize {
        let batches = fixture.batches.len();
        let files = fixture
            .batches
            .iter()
            .map(|batch| batch.entry.files.len())
            .sum::<usize>();
        let mut dates: Vec<TradingDate> = Vec::new();
        for batch in &fixture.batches {
            if !dates.contains(&batch.entry.date) {
                dates.push(batch.entry.date);
            }
        }
        let components = component_count(&fixture.data_root);
        if batches == 0 {
            components + 7
        } else {
            components + 9 + dates.len() + 2 * batches + files
        }
    }

    fn body_path(fixture: &Fixture, batch_index: usize, file_index: usize) -> PathBuf {
        let batch = &fixture.batches[batch_index];
        batch_dir(&fixture.data_root, &batch.entry).join(&batch.entry.files[file_index].file_name)
    }

    fn metadata_path(fixture: &Fixture, batch_index: usize) -> PathBuf {
        batch_dir(&fixture.data_root, &fixture.batches[batch_index].entry).join("batch.json")
    }

    fn rewrite_batch_metadata(fixture: &Fixture, batch_index: usize, entry: &ManifestEntry) {
        private_file(
            &metadata_path(fixture, batch_index),
            &serde_json::to_vec_pretty(entry).unwrap(),
        );
    }

    #[test]
    fn s2_t01_valid_multi_batch_snapshot_keeps_exact_evidence_and_stats() {
        let fixture = fixture();
        let manifest = fs::read(manifest_path(&fixture.data_root)).unwrap();
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.scope().provider(), "kis");
        assert_eq!(snapshot.scope().market(), "kr");
        assert_eq!(snapshot.manifest_bytes(), manifest.as_slice());
        assert_eq!(
            snapshot.manifest_sha256(),
            &ContentHash::from_bytes(&manifest)
        );
        assert_eq!(snapshot.batches().len(), 2);
        assert_eq!(
            snapshot.batches()[0].batch_id(),
            fixture.batches[0].entry.batch_id
        );
        assert_eq!(
            snapshot.batches()[1].batch_id(),
            fixture.batches[1].entry.batch_id
        );
        assert_eq!(
            snapshot.batches()[0].retrieved_at(),
            fixture.batches[0].entry.retrieved_at
        );
        assert_eq!(snapshot.batches()[0].mode(), FetchMode::Synthetic);
        assert_eq!(
            snapshot.batches()[0].manifest_line_bytes(),
            serde_json::to_vec(&fixture.batches[0].entry).unwrap()
        );
        let file = snapshot.batches()[0].files().next().unwrap();
        assert_eq!(file.file_name(), "bars.json");
        assert_eq!(file.kind(), ResponseKind::Bars);
        assert_eq!(
            file.size_bytes(),
            u64::try_from(DEFAULT_BODY.len()).unwrap()
        );
        assert_eq!(file.bytes(), DEFAULT_BODY);
        assert_eq!(
            file.content_hash(),
            &fixture.batches[0].entry.files[0].content_hash
        );
        assert_eq!(
            snapshot.batches()[0].batch_json_sha256(),
            &ContentHash::from_bytes(snapshot.batches()[0].batch_json_bytes())
        );
        let stats: SnapshotStats = snapshot.stats();
        assert_eq!(stats.manifest_bytes, manifest.len());
        assert_eq!(stats.manifest_records, 2);
        assert_eq!(stats.batches, 2);
        assert_eq!(stats.files, 3);
        let retained = manifest.len()
            + fixture
                .batches
                .iter()
                .map(|batch| {
                    fs::metadata(metadata_path(
                        &fixture,
                        fixture
                            .batches
                            .iter()
                            .position(|item| item.entry.batch_id == batch.entry.batch_id)
                            .unwrap(),
                    ))
                    .unwrap()
                    .len() as usize
                        + batch
                            .bodies
                            .iter()
                            .map(|(_, body)| body.len())
                            .sum::<usize>()
                })
                .sum::<usize>();
        assert_eq!(stats.retained_evidence_bytes, retained);
        assert_eq!(stats.open_descriptors, fd_bound(&fixture));
        assert_eq!(stats.peak_open_descriptors, fd_bound(&fixture));
    }

    #[test]
    fn s2_t02_line_bytes_keep_whitespace_crlf_order_and_physical_line() {
        let fixture = fixture_with(vec![
            batch(1, "2026-10-13", &[("bars.json", DEFAULT_BODY)]),
            batch(2, "2026-10-14", &[("next.json", b"next")]),
        ]);
        valid_control(&fixture);
        let s1 = RawStore::new(fixture.data_root.clone())
            .read_committed_manifest_evidence(
                "kis",
                "kr",
                fixture.uid,
                CommittedManifestReadLimits {
                    max_manifest_bytes: 65_536,
                    max_records: 16,
                },
            )
            .unwrap();
        assert_eq!(s1.len(), 2);
        for (index, source_batch) in fixture.batches.iter().enumerate() {
            assert_eq!(s1[index].entry(), &source_batch.entry);
            assert_eq!(
                s1[index].line_bytes(),
                serde_json::to_vec(&source_batch.entry).unwrap()
            );
            assert_eq!(s1[index].line_number(), index + 1);
        }
        let entry = &fixture.batches[0].entry;
        let first = reversed_top_level_json(entry);
        let mut manifest = b" \t".to_vec();
        manifest.extend_from_slice(&first);
        manifest.extend_from_slice(b" \r\n\n");
        let second_batch = &fixture.batches[1];
        let second = serde_json::to_vec(&second_batch.entry).unwrap();
        manifest.extend_from_slice(&second);
        manifest.extend_from_slice(b"\n");
        rewrite_manifest(&fixture, &manifest);

        let snapshot = scan(&fixture, limits()).unwrap();
        let record = &snapshot.batches()[0];
        let exact_line = [&b" \t"[..], &first[..], &b" \r"[..]].concat();
        assert_eq!(record.manifest_line_bytes(), exact_line.as_slice());
        assert_eq!(
            record.manifest_line_sha256(),
            &ContentHash::from_bytes(&exact_line)
        );
        assert_eq!(record.manifest_line_number(), 1);
        assert_eq!(snapshot.batches()[1].manifest_line_number(), 3);
        assert_eq!(
            snapshot.batches()[1].manifest_line_bytes(),
            second.as_slice()
        );
        assert_eq!(
            snapshot.manifest_sha256(),
            &ContentHash::from_bytes(&manifest)
        );
        assert_eq!(snapshot.stats().manifest_records, 2);
    }

    #[test]
    fn s2_t03_final_record_eof_tail_and_terminated_errors_are_distinct() {
        let fixture = one_batch();
        valid_control(&fixture);
        let complete = serde_json::to_vec(&fixture.batches[0].entry).unwrap();
        rewrite_manifest(&fixture, &complete);
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.stats().manifest_records, 1);
        drop(snapshot);

        let mut with_torn_tail = complete.clone();
        with_torn_tail.extend_from_slice(b"\n{\"batch_id\":");
        rewrite_manifest(&fixture, &with_torn_tail);
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.stats().manifest_records, 1);
        assert_eq!(snapshot.manifest_bytes(), with_torn_tail.as_slice());
        drop(snapshot);

        with_torn_tail.push(b'\n');
        rewrite_manifest(&fixture, &with_torn_tail);
        assert_eq!(
            error(&fixture, limits()),
            DailyEvidenceError::InvalidManifest
        );

        rewrite_manifest(&fixture, b"{\"provider\":\"kis\",\"market\":]\n");
        assert_eq!(
            error(&fixture, limits()),
            DailyEvidenceError::InvalidManifest
        );
    }

    #[test]
    fn s2_t03_known_duplicate_fields_reject_before_an_incomplete_second_value() {
        let fixture = one_batch();
        let mut row_value = serde_json::to_value(&fixture.batches[0].entry).unwrap();
        row_value
            .as_object_mut()
            .unwrap()
            .insert("entitlement_reference".to_owned(), serde_json::Value::Null);
        let valid_row = serde_json::to_vec(&row_value).unwrap();
        let mut valid_manifest = valid_row.clone();
        valid_manifest.push(b'\n');
        rewrite_manifest(&fixture, &valid_manifest);
        valid_control(&fixture);
        let legacy = RawStore::new(&fixture.data_root);
        assert!(legacy.read_committed_manifest("kis", "kr").is_ok());

        for field in [
            "batch_id",
            "provider",
            "market",
            "date",
            "retrieved_at",
            "mode",
            "entitlement_reference",
            "files",
        ] {
            rewrite_manifest(&fixture, &valid_manifest);
            valid_control(&fixture);
            assert!(legacy.read_committed_manifest("kis", "kr").is_ok());

            let mut duplicate = valid_row.clone();
            assert_eq!(duplicate.pop(), Some(b'}'));
            duplicate.extend_from_slice(format!(",\"{field}\":").as_bytes());
            let mut manifest = valid_manifest.clone();
            manifest.extend_from_slice(&duplicate);
            rewrite_manifest(&fixture, &manifest);
            assert!(legacy.read_committed_manifest("kis", "kr").is_err());
            assert_eq!(
                error(&fixture, limits()),
                DailyEvidenceError::InvalidManifest
            );
        }
    }

    #[test]
    fn s2_t04_equal_duplicates_keep_first_line_and_conflicts_fail_closed() {
        let fixture = one_batch();
        valid_control(&fixture);
        let first = serde_json::to_vec(&fixture.batches[0].entry).unwrap();
        let mut manifest = first.clone();
        manifest.extend_from_slice(b"\n  ");
        manifest.extend_from_slice(&first);
        manifest.push(b'\n');
        rewrite_manifest(&fixture, &manifest);
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.stats().manifest_records, 2);
        assert_eq!(snapshot.batches().len(), 1);
        assert_eq!(snapshot.stats().files, 1);
        assert_eq!(
            snapshot.batches()[0].manifest_line_bytes(),
            first.as_slice()
        );
        assert_eq!(snapshot.batches()[0].manifest_line_number(), 1);
        let retained_once = snapshot.manifest_bytes().len()
            + snapshot.batches()[0].batch_json_bytes().len()
            + snapshot.batches()[0]
                .files()
                .map(|file| file.bytes().len())
                .sum::<usize>();
        assert_eq!(snapshot.stats().retained_evidence_bytes, retained_once);
        drop(snapshot);

        let exact_duplicate_cap = custom_limits(|values| values.max_retained_bytes = retained_once);
        let snapshot = scan(&fixture, exact_duplicate_cap).unwrap();
        assert_eq!(
            snapshot.stats().retained_evidence_bytes,
            retained_once,
            "the duplicate manifest row is retained as manifest bytes, while the batch is captured once"
        );
        drop(snapshot);
        let under_duplicate_cap =
            custom_limits(|values| values.max_retained_bytes = retained_once - 1);
        assert_eq!(
            error(&fixture, under_duplicate_cap),
            DailyEvidenceError::SourceScanLimit
        );

        let mut changed = fixture.batches[0].entry.clone();
        changed.entitlement_reference = Some("changed".to_owned());
        let mut conflict = first;
        conflict.push(b'\n');
        conflict.extend(serde_json::to_vec(&changed).unwrap());
        rewrite_manifest(&fixture, &conflict);
        assert_eq!(
            error(&fixture, limits()),
            DailyEvidenceError::ManifestConflict
        );
    }

    #[test]
    fn s2_t05_manifest_byte_and_pre_dedup_record_caps_are_exact() {
        let fixture = one_batch();
        valid_control(&fixture);
        let row = serde_json::to_vec(&fixture.batches[0].entry).unwrap();
        rewrite_manifest(&fixture, &row);
        let exact = row.len();
        let bounded = custom_limits(|values| {
            values.max_manifest_bytes = exact;
            values.max_manifest_records = 1;
        });
        assert_eq!(scan(&fixture, bounded).unwrap().stats().manifest_records, 1);

        let byte_over = custom_limits(|values| values.max_manifest_bytes = exact - 1);
        assert_eq!(
            error(&fixture, byte_over),
            DailyEvidenceError::SourceScanLimit
        );

        let mut duplicate = row.clone();
        duplicate.push(b'\n');
        duplicate.extend_from_slice(&row);
        rewrite_manifest(&fixture, &duplicate);
        let record_cap = custom_limits(|values| values.max_manifest_records = 1);
        assert_eq!(
            error(&fixture, record_cap),
            DailyEvidenceError::SourceScanLimit
        );

        let mut torn_tail = row.clone();
        torn_tail.extend_from_slice(b"\n{\"batch_id\":");
        rewrite_manifest(&fixture, &torn_tail);
        let tail_bytes = custom_limits(|values| values.max_manifest_bytes = row.len());
        assert_eq!(
            error(&fixture, tail_bytes),
            DailyEvidenceError::SourceScanLimit
        );

        rewrite_manifest(&fixture, &row);
        let final_unterminated = custom_limits(|values| values.max_manifest_records = 1);
        assert_eq!(
            scan(&fixture, final_unterminated)
                .unwrap()
                .stats()
                .manifest_records,
            1
        );
    }

    #[test]
    fn s2_t06_zero_overflow_and_policy_invalid_limits_fail_in_construction() {
        let zeroed: [fn(&mut ScanLimitValues); 12] = [
            |v: &mut ScanLimitValues| v.max_manifest_bytes = 0,
            |v: &mut ScanLimitValues| v.max_manifest_records = 0,
            |v: &mut ScanLimitValues| v.max_batches = 0,
            |v: &mut ScanLimitValues| v.max_files_per_batch = 0,
            |v: &mut ScanLimitValues| v.max_files = 0,
            |v: &mut ScanLimitValues| v.max_batch_metadata_bytes = 0,
            |v: &mut ScanLimitValues| v.max_source_file_bytes = 0,
            |v: &mut ScanLimitValues| v.max_retained_bytes = 0,
            |v: &mut ScanLimitValues| v.max_open_descriptors = 0,
            |v: &mut ScanLimitValues| v.max_data_root_bytes = 0,
            |v: &mut ScanLimitValues| v.max_data_root_components = 0,
            |v: &mut ScanLimitValues| v.max_metadata_json_depth = 0,
        ];
        for set_zero in zeroed {
            let mut each = values();
            set_zero(&mut each);
            assert_eq!(
                ScanLimits::new(each).err(),
                Some(DailyEvidenceError::InvalidLimits)
            );
        }
        let mut each = values();
        each.max_manifest_bytes = usize::MAX;
        assert_eq!(
            ScanLimits::new(each).err(),
            Some(DailyEvidenceError::ArithmeticOverflow)
        );
        each = values();
        each.max_manifest_records = usize::MAX - 1;
        assert_eq!(
            ScanLimits::new(each).err(),
            Some(DailyEvidenceError::ArithmeticOverflow)
        );
        each = values();
        each.max_source_file_bytes = 67_108_864;
        assert!(ScanLimits::new(each).is_ok());
        each.max_source_file_bytes = 67_108_865;
        assert_eq!(
            ScanLimits::new(each).err(),
            Some(DailyEvidenceError::InvalidLimits)
        );
        each = values();
        each.max_metadata_json_depth = 33;
        assert_eq!(
            ScanLimits::new(each).err(),
            Some(DailyEvidenceError::InvalidLimits)
        );
    }

    #[test]
    fn s2_t07_batch_metadata_and_body_byte_caps_precede_capture() {
        let fixture = one_batch();
        valid_control(&fixture);
        let metadata_len = fs::metadata(metadata_path(&fixture, 0)).unwrap().len() as usize;
        let body_len = fixture.batches[0].bodies[0].1.len();
        let exact = custom_limits(|values| {
            values.max_batch_metadata_bytes = metadata_len;
            values.max_source_file_bytes = body_len;
        });
        assert!(scan(&fixture, exact).is_ok());

        let metadata_over = custom_limits(|values| {
            values.max_batch_metadata_bytes = metadata_len - 1;
        });
        assert_eq!(
            error(&fixture, metadata_over),
            DailyEvidenceError::SourceScanLimit
        );

        let body_over = custom_limits(|values| values.max_source_file_bytes = body_len - 1);
        assert_eq!(
            error(&fixture, body_over),
            DailyEvidenceError::SourceScanLimit
        );
    }

    #[test]
    fn s2_t08_per_batch_and_total_file_counts_fail_before_body_open() {
        let fixture = fixture();
        valid_control(&fixture);
        let per_batch = custom_limits(|values| values.max_files_per_batch = 1);
        assert_eq!(
            error(&fixture, per_batch),
            DailyEvidenceError::SourceScanLimit
        );
        let total = custom_limits(|values| values.max_files = 2);
        assert_eq!(error(&fixture, total), DailyEvidenceError::SourceScanLimit);
        let unique_batches = custom_limits(|values| values.max_batches = 1);
        assert_eq!(
            error(&fixture, unique_batches),
            DailyEvidenceError::SourceScanLimit
        );

        let small_fields = one_batch();
        valid_control(&small_fields);
        let mut many_fields = small_fields.batches[0].entry.clone();
        many_fields.files[0].request.query = (0..64)
            .map(|index| (format!("k{index}"), String::new()))
            .collect();
        rewrite_manifest(&small_fields, &serde_json::to_vec(&many_fields).unwrap());
        rewrite_batch_metadata(&small_fields, 0, &many_fields);
        valid_control(&small_fields);
        let metadata_len = fs::metadata(metadata_path(&small_fields, 0)).unwrap().len() as usize;
        let exact_metadata = custom_limits(|values| values.max_batch_metadata_bytes = metadata_len);
        assert!(scan(&small_fields, exact_metadata).is_ok());
        let bounded_metadata =
            custom_limits(|values| values.max_batch_metadata_bytes = metadata_len - 1);
        assert_eq!(
            error(&small_fields, bounded_metadata),
            DailyEvidenceError::SourceScanLimit
        );
    }

    #[test]
    fn s2_t09_aggregate_retained_bytes_accept_exact_and_reject_one_under() {
        let fixture = fixture_with(vec![batch(
            1,
            "2026-10-13",
            &[("bars.json", b"bars"), ("actions.json", b"actions")],
        )]);
        valid_control(&fixture);
        let manifest_len = fs::metadata(manifest_path(&fixture.data_root))
            .unwrap()
            .len() as usize;
        let metadata_len = fs::metadata(metadata_path(&fixture, 0)).unwrap().len() as usize;
        let body_bytes = fixture.batches[0]
            .bodies
            .iter()
            .map(|(_, body)| body.len())
            .sum::<usize>();
        let exact_total = manifest_len + metadata_len + body_bytes;
        let exact = custom_limits(|values| values.max_retained_bytes = exact_total);
        assert_eq!(
            scan(&fixture, exact)
                .unwrap()
                .stats()
                .retained_evidence_bytes,
            exact_total
        );
        let under = custom_limits(|values| values.max_retained_bytes = exact_total - 1);
        assert_eq!(error(&fixture, under), DailyEvidenceError::SourceScanLimit);
    }

    #[test]
    fn s2_t10_descriptor_formula_is_measured_and_preflighted() {
        let fixture = fixture();
        valid_control(&fixture);
        let required = fd_bound(&fixture);
        let exact = custom_limits(|values| values.max_open_descriptors = required);
        let snapshot = scan(&fixture, exact).unwrap();
        assert_eq!(snapshot.stats().open_descriptors, required);
        assert_eq!(snapshot.stats().peak_open_descriptors, required);
        drop(snapshot);
        let one_under = custom_limits(|values| values.max_open_descriptors = required - 1);
        assert_eq!(
            error(&fixture, one_under),
            DailyEvidenceError::SourceScanLimit
        );

        let shared_date = fixture_with(vec![
            batch(3, "2026-10-13", &[("first.json", b"first")]),
            batch(4, "2026-10-13", &[("second.json", b"second")]),
        ]);
        valid_control(&shared_date);
        let shared_required = fd_bound(&shared_date);
        let shared_exact = custom_limits(|values| values.max_open_descriptors = shared_required);
        let snapshot = scan(&shared_date, shared_exact).unwrap();
        assert_eq!(snapshot.stats().open_descriptors, shared_required);
        assert_eq!(snapshot.stats().peak_open_descriptors, shared_required);

        let mut deep_root = one_batch();
        let old_root = deep_root.data_root.clone();
        let new_root = deep_root._temp.path().join("one/two/three/data");
        fs::create_dir_all(new_root.parent().unwrap()).unwrap();
        fs::rename(old_root, &new_root).unwrap();
        deep_root.data_root = new_root;
        valid_control(&deep_root);
        let deep_required = fd_bound(&deep_root);
        let deep_exact = custom_limits(|values| values.max_open_descriptors = deep_required);
        let snapshot = scan(&deep_root, deep_exact).unwrap();
        assert_eq!(snapshot.stats().open_descriptors, deep_required);
        assert_eq!(snapshot.stats().peak_open_descriptors, deep_required);
    }

    fn with_nested_unknown(entry: &ManifestEntry, arrays: usize) -> Vec<u8> {
        let mut value = serde_json::to_value(entry).unwrap();
        let mut nested = serde_json::Value::Null;
        for _ in 0..arrays {
            nested = serde_json::Value::Array(vec![nested]);
        }
        value
            .as_object_mut()
            .unwrap()
            .insert("future_raw_field".to_owned(), nested);
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn s2_t11_root_path_and_unknown_json_depth_caps_are_exact() {
        let fixture = one_batch();
        valid_control(&fixture);
        let line = with_nested_unknown(&fixture.batches[0].entry, 31);
        rewrite_manifest(&fixture, &line);
        let exact_depth = custom_limits(|values| {
            values.max_metadata_json_depth = 32;
            values.max_data_root_bytes = fixture.data_root.as_os_str().as_bytes().len();
            values.max_data_root_components = component_count(&fixture.data_root);
        });
        assert!(scan(&fixture, exact_depth).is_ok());

        let over_depth = with_nested_unknown(&fixture.batches[0].entry, 32);
        rewrite_manifest(&fixture, &over_depth);
        let depth_limit = custom_limits(|values| values.max_metadata_json_depth = 32);
        assert_eq!(
            error(&fixture, depth_limit),
            DailyEvidenceError::SourceScanLimit
        );

        rewrite_manifest(&fixture, &line);
        valid_control(&fixture);
        let root_over = custom_limits(|values| {
            values.max_data_root_bytes = fixture.data_root.as_os_str().as_bytes().len() - 1;
        });
        assert_eq!(
            error(&fixture, root_over),
            DailyEvidenceError::SourceScanLimit
        );

        rewrite_manifest(&fixture, &line);
        valid_control(&fixture);
        let component_over = custom_limits(|values| {
            values.max_data_root_components = component_count(&fixture.data_root) - 1;
        });
        assert_eq!(
            error(&fixture, component_over),
            DailyEvidenceError::SourceScanLimit
        );
    }

    #[test]
    fn s2_t12_missing_controls_do_not_repair_and_empty_manifest_is_valid() {
        let fixture = one_batch();
        valid_control(&fixture);
        fs::remove_file(lock_path(&fixture.data_root)).unwrap();
        assert_eq!(
            error(&fixture, limits()),
            DailyEvidenceError::MissingControl
        );
        assert!(!lock_path(&fixture.data_root).exists());

        let missing_manifest = one_batch();
        valid_control(&missing_manifest);
        fs::remove_file(manifest_path(&missing_manifest.data_root)).unwrap();
        assert_eq!(
            error(&missing_manifest, limits()),
            DailyEvidenceError::MissingControl
        );
        assert!(!manifest_path(&missing_manifest.data_root).exists());

        let missing_parent = one_batch();
        valid_control(&missing_parent);
        fs::remove_dir_all(manifest_dir(&missing_parent.data_root)).unwrap();
        assert_eq!(
            error(&missing_parent, limits()),
            DailyEvidenceError::MissingControl
        );
        assert!(!manifest_dir(&missing_parent.data_root).exists());

        let missing_raw = one_batch();
        valid_control(&missing_raw);
        fs::remove_dir_all(raw_root(&missing_raw.data_root)).unwrap();
        assert_eq!(
            error(&missing_raw, limits()),
            DailyEvidenceError::MissingControl
        );
        assert!(!raw_root(&missing_raw.data_root).exists());

        let empty = empty_fixture();
        valid_control(&empty);
        let snapshot = scan(&empty, limits()).unwrap();
        assert!(snapshot.batches().is_empty());
        assert!(snapshot.manifest_bytes().is_empty());
        assert_eq!(snapshot.stats().open_descriptors, fd_bound(&empty));
    }

    #[test]
    fn s2_t13_root_control_batch_and_leaf_symlinks_fail_closed() {
        let root_fixture = one_batch();
        valid_control(&root_fixture);
        let root_link = root_fixture._temp.path().join("data-link");
        symlink(&root_fixture.data_root, &root_link).unwrap();
        assert_eq!(
            CommittedRawReader::new(&root_link)
                .snapshot(scope(&root_fixture), limits())
                .err()
                .unwrap(),
            DailyEvidenceError::UnsafePath
        );

        let parent_fixture = one_batch();
        valid_control(&parent_fixture);
        let alias = parent_fixture._temp.path().join("data-parent-alias");
        symlink(parent_fixture._temp.path(), &alias).unwrap();
        let aliased_root = alias.join("data");
        assert_eq!(
            CommittedRawReader::new(&aliased_root)
                .snapshot(scope(&parent_fixture), limits())
                .err(),
            Some(DailyEvidenceError::UnsafePath)
        );

        let parent_component = one_batch();
        valid_control(&parent_component);
        let parent_root = parent_component.data_root.join("..").join("data");
        assert_eq!(
            CommittedRawReader::new(&parent_root)
                .snapshot(scope(&parent_component), limits())
                .err(),
            Some(DailyEvidenceError::UnsafePath)
        );

        let control_fixture = one_batch();
        valid_control(&control_fixture);
        let original = manifest_dir(&control_fixture.data_root);
        let moved = control_fixture._temp.path().join("manifest-market");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&control_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let manifests_fixture = one_batch();
        valid_control(&manifests_fixture);
        let original = raw_root(&manifests_fixture.data_root).join("manifests");
        let moved = manifests_fixture._temp.path().join("saved-manifests");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&manifests_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let manifest_provider_fixture = one_batch();
        valid_control(&manifest_provider_fixture);
        let original = raw_root(&manifest_provider_fixture.data_root)
            .join("manifests")
            .join("provider=kis");
        let moved = manifest_provider_fixture
            ._temp
            .path()
            .join("saved-manifest-provider");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&manifest_provider_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let manifest_link_fixture = one_batch();
        valid_control(&manifest_link_fixture);
        let manifest = manifest_path(&manifest_link_fixture.data_root);
        let moved_manifest = manifest_link_fixture._temp.path().join("saved-manifest");
        fs::rename(&manifest, &moved_manifest).unwrap();
        symlink(&moved_manifest, &manifest).unwrap();
        assert_eq!(
            error(&manifest_link_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let lock_link_fixture = one_batch();
        valid_control(&lock_link_fixture);
        let lock = lock_path(&lock_link_fixture.data_root);
        let moved_lock = lock_link_fixture._temp.path().join("saved-lock");
        fs::rename(&lock, &moved_lock).unwrap();
        symlink(&moved_lock, &lock).unwrap();
        assert_eq!(
            error(&lock_link_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let leaf_fixture = one_batch();
        valid_control(&leaf_fixture);
        let leaf = body_path(&leaf_fixture, 0, 0);
        let elsewhere = leaf_fixture._temp.path().join("outside-body");
        fs::write(&elsewhere, DEFAULT_BODY).unwrap();
        fs::remove_file(&leaf).unwrap();
        symlink(&elsewhere, &leaf).unwrap();
        assert_eq!(
            error(&leaf_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let batch_fixture = one_batch();
        valid_control(&batch_fixture);
        let batch_directory = batch_dir(&batch_fixture.data_root, &batch_fixture.batches[0].entry);
        let moved_batch = batch_fixture._temp.path().join("saved-batch");
        fs::rename(&batch_directory, &moved_batch).unwrap();
        symlink(&moved_batch, &batch_directory).unwrap();
        assert_eq!(
            error(&batch_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let batch_json_fixture = one_batch();
        valid_control(&batch_json_fixture);
        let original = metadata_path(&batch_json_fixture, 0);
        let moved = batch_json_fixture._temp.path().join("saved-batch-json");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&batch_json_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let raw_fixture = one_batch();
        valid_control(&raw_fixture);
        let raw = raw_root(&raw_fixture.data_root);
        let moved_raw = raw_fixture._temp.path().join("saved-raw");
        fs::rename(&raw, &moved_raw).unwrap();
        symlink(&moved_raw, &raw).unwrap();
        assert_eq!(
            error(&raw_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let data_provider_fixture = one_batch();
        valid_control(&data_provider_fixture);
        let original = raw_root(&data_provider_fixture.data_root).join("provider=kis");
        let moved = data_provider_fixture
            ._temp
            .path()
            .join("saved-data-provider");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&data_provider_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let data_market_fixture = one_batch();
        valid_control(&data_market_fixture);
        let original = raw_root(&data_market_fixture.data_root)
            .join("provider=kis")
            .join("market=kr");
        let moved = data_market_fixture._temp.path().join("saved-data-market");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        assert_eq!(
            error(&data_market_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let date_fixture = one_batch();
        valid_control(&date_fixture);
        let date_directory = raw_root(&date_fixture.data_root)
            .join("provider=kis")
            .join("market=kr")
            .join(format!("date={}", date_fixture.batches[0].entry.date));
        let moved_date = date_fixture._temp.path().join("saved-date");
        fs::rename(&date_directory, &moved_date).unwrap();
        symlink(&moved_date, &date_directory).unwrap();
        assert_eq!(
            error(&date_fixture, limits()),
            DailyEvidenceError::UnsafePath
        );

        let ordinary_file_fixture = one_batch();
        valid_control(&ordinary_file_fixture);
        let ordinary = ordinary_file_fixture._temp.path().join("ordinary-file");
        private_file(&ordinary, b"not a directory");
        let ordinary_root = ordinary.join("data");
        assert_eq!(
            CommittedRawReader::new(&ordinary_root)
                .snapshot(scope(&ordinary_file_fixture), limits())
                .err(),
            Some(DailyEvidenceError::UnsafePath)
        );
    }

    #[test]
    fn s2_t14_special_nodes_owner_and_mode_guards_do_not_block_or_bypass() {
        let fifo_fixture = one_batch();
        valid_control(&fifo_fixture);
        let fifo = body_path(&fifo_fixture, 0, 0);
        fs::remove_file(&fifo).unwrap();
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            fifo.as_path(),
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        assert_eq!(
            error(&fifo_fixture, limits()),
            DailyEvidenceError::NonRegularFile
        );

        let directory_fixture = one_batch();
        valid_control(&directory_fixture);
        let body = body_path(&directory_fixture, 0, 0);
        fs::remove_file(&body).unwrap();
        private_dir(&body);
        assert_eq!(
            error(&directory_fixture, limits()),
            DailyEvidenceError::NonRegularFile
        );

        let writable_fixture = one_batch();
        valid_control(&writable_fixture);
        set_mode(&body_path(&writable_fixture, 0, 0), 0o660);
        assert_eq!(
            error(&writable_fixture, limits()),
            DailyEvidenceError::WritableNode
        );

        let writable_directory = one_batch();
        valid_control(&writable_directory);
        set_mode(
            &batch_dir(
                &writable_directory.data_root,
                &writable_directory.batches[0].entry,
            ),
            0o770,
        );
        assert_eq!(
            error(&writable_directory, limits()),
            DailyEvidenceError::WritableNode
        );

        let uid_zero = ApprovedRawScope::new("kis", "kr", 0).unwrap();
        assert_eq!(uid_zero.trusted_raw_owner_uid(), 0);
        let invalid_uid = ApprovedRawScope::new("kis", "kr", u32::MAX);
        assert_eq!(invalid_uid.err(), Some(DailyEvidenceError::InvalidScope));

        let wrong_uid_fixture = one_batch();
        valid_control(&wrong_uid_fixture);
        let expected_other_uid = wrong_uid_fixture.uid.checked_add(1).unwrap();
        let wrong_owner = ApprovedRawScope::new("kis", "kr", expected_other_uid).unwrap();
        assert_eq!(
            CommittedRawReader::new(&wrong_uid_fixture.data_root)
                .snapshot(wrong_owner, limits())
                .err(),
            Some(DailyEvidenceError::UntrustedOwner)
        );
    }

    #[test]
    fn s2_t15_hardlinked_lock_manifest_metadata_and_body_are_rejected() {
        let body_fixture = one_batch();
        valid_control(&body_fixture);
        let body = body_path(&body_fixture, 0, 0);
        hard_link(&body, body_fixture._temp.path().join("body-link")).unwrap();
        assert_eq!(
            error(&body_fixture, limits()),
            DailyEvidenceError::HardLinkedFile
        );

        let manifest_fixture = one_batch();
        valid_control(&manifest_fixture);
        hard_link(
            manifest_path(&manifest_fixture.data_root),
            manifest_fixture._temp.path().join("manifest-link"),
        )
        .unwrap();
        assert_eq!(
            error(&manifest_fixture, limits()),
            DailyEvidenceError::HardLinkedFile
        );

        let lock_fixture = one_batch();
        valid_control(&lock_fixture);
        hard_link(
            lock_path(&lock_fixture.data_root),
            lock_fixture._temp.path().join("lock-link"),
        )
        .unwrap();
        assert_eq!(
            error(&lock_fixture, limits()),
            DailyEvidenceError::HardLinkedFile
        );

        let metadata_fixture = one_batch();
        valid_control(&metadata_fixture);
        hard_link(
            &metadata_path(&metadata_fixture, 0),
            metadata_fixture._temp.path().join("metadata-link"),
        )
        .unwrap();
        assert_eq!(
            error(&metadata_fixture, limits()),
            DailyEvidenceError::HardLinkedFile
        );
    }

    #[test]
    fn s2_t16_scope_and_complete_batch_metadata_equality_are_required() {
        let scope_fixture = one_batch();
        valid_control(&scope_fixture);
        let mut wrong_scope = scope_fixture.batches[0].entry.clone();
        wrong_scope.provider = "other-provider".to_owned();
        rewrite_manifest(&scope_fixture, &serde_json::to_vec(&wrong_scope).unwrap());
        assert_eq!(
            error(&scope_fixture, limits()),
            DailyEvidenceError::ScopeMismatch
        );

        let market_fixture = one_batch();
        valid_control(&market_fixture);
        let mut wrong_market = market_fixture.batches[0].entry.clone();
        wrong_market.market = "other-market".to_owned();
        rewrite_manifest(&market_fixture, &serde_json::to_vec(&wrong_market).unwrap());
        assert_eq!(
            error(&market_fixture, limits()),
            DailyEvidenceError::ScopeMismatch
        );

        let metadata_fixture = one_batch();
        valid_control(&metadata_fixture);
        let mut changed_metadata = metadata_fixture.batches[0].entry.clone();
        changed_metadata.date = TradingDate::parse("2026-10-12").unwrap();
        rewrite_batch_metadata(&metadata_fixture, 0, &changed_metadata);
        assert_eq!(
            error(&metadata_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let batch_id_fixture = one_batch();
        valid_control(&batch_id_fixture);
        let mut changed_id = batch_id_fixture.batches[0].entry.clone();
        changed_id.batch_id = BatchId::from_uuid(Uuid::from_u128(2));
        rewrite_batch_metadata(&batch_id_fixture, 0, &changed_id);
        assert_eq!(
            error(&batch_id_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let request_fixture = one_batch();
        valid_control(&request_fixture);
        let mut changed_request = request_fixture.batches[0].entry.clone();
        changed_request.files[0].request.endpoint = "different".to_owned();
        rewrite_batch_metadata(&request_fixture, 0, &changed_request);
        assert_eq!(
            error(&request_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let time_fixture = one_batch();
        valid_control(&time_fixture);
        let mut changed_time = time_fixture.batches[0].entry.clone();
        changed_time.retrieved_at = UtcTimestamp::parse_rfc3339("2026-10-14T01:02:04Z").unwrap();
        rewrite_batch_metadata(&time_fixture, 0, &changed_time);
        assert_eq!(
            error(&time_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let mode_fixture = one_batch();
        valid_control(&mode_fixture);
        let mut changed_mode = mode_fixture.batches[0].entry.clone();
        changed_mode.mode = FetchMode::Credentialed;
        rewrite_batch_metadata(&mode_fixture, 0, &changed_mode);
        assert_eq!(
            error(&mode_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let size_fixture = one_batch();
        valid_control(&size_fixture);
        let mut changed_size = size_fixture.batches[0].entry.clone();
        changed_size.files[0].size_bytes += 1;
        rewrite_batch_metadata(&size_fixture, 0, &changed_size);
        assert_eq!(
            error(&size_fixture, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );

        let file_order = fixture_with(vec![batch(
            7,
            "2026-10-13",
            &[("first.json", b"1"), ("second.json", b"2")],
        )]);
        valid_control(&file_order);
        let mut reordered = file_order.batches[0].entry.clone();
        reordered.files.reverse();
        rewrite_batch_metadata(&file_order, 0, &reordered);
        assert_eq!(
            error(&file_order, limits()),
            DailyEvidenceError::BatchMetadataMismatch
        );
    }

    #[test]
    fn s2_t17_duplicate_reserved_traversal_and_extra_entries_fail() {
        let duplicate = one_batch();
        valid_control(&duplicate);
        let mut entry = duplicate.batches[0].entry.clone();
        entry.files.push(entry.files[0].clone());
        rewrite_manifest(&duplicate, &serde_json::to_vec(&entry).unwrap());
        assert_eq!(
            error(&duplicate, limits()),
            DailyEvidenceError::DuplicateFile
        );

        let reserved = one_batch();
        valid_control(&reserved);
        let mut entry = reserved.batches[0].entry.clone();
        entry.files[0].file_name = "batch.json".to_owned();
        rewrite_manifest(&reserved, &serde_json::to_vec(&entry).unwrap());
        assert_eq!(
            error(&reserved, limits()),
            DailyEvidenceError::InvalidFileName
        );

        let traversal = one_batch();
        valid_control(&traversal);
        let mut entry = traversal.batches[0].entry.clone();
        entry.files[0].file_name = "../escape.json".to_owned();
        rewrite_manifest(&traversal, &serde_json::to_vec(&entry).unwrap());
        assert_eq!(
            error(&traversal, limits()),
            DailyEvidenceError::InvalidFileName
        );

        let extra = one_batch();
        valid_control(&extra);
        private_file(
            &batch_dir(&extra.data_root, &extra.batches[0].entry).join(".leftover.tmp"),
            b"x",
        );
        assert_eq!(
            error(&extra, limits()),
            DailyEvidenceError::UnexpectedBatchEntry
        );

        let extra_directory = one_batch();
        valid_control(&extra_directory);
        private_dir(
            &batch_dir(
                &extra_directory.data_root,
                &extra_directory.batches[0].entry,
            )
            .join("unlisted-subdirectory"),
        );
        assert_eq!(
            error(&extra_directory, limits()),
            DailyEvidenceError::UnexpectedBatchEntry
        );
    }

    #[test]
    fn s2_t18_missing_invalid_zero_size_and_same_size_corruption_reject() {
        let missing = one_batch();
        valid_control(&missing);
        fs::remove_file(body_path(&missing, 0, 0)).unwrap();
        assert_eq!(
            error(&missing, limits()),
            DailyEvidenceError::MissingCommittedEvidence
        );

        let zero = one_batch();
        valid_control(&zero);
        let mut entry = zero.batches[0].entry.clone();
        entry.files[0].size_bytes = 0;
        rewrite_manifest(&zero, &serde_json::to_vec(&entry).unwrap());
        rewrite_batch_metadata(&zero, 0, &entry);
        assert_eq!(error(&zero, limits()), DailyEvidenceError::SizeMismatch);

        let corrupt = one_batch();
        valid_control(&corrupt);
        let original = corrupt.batches[0].bodies[0].1.clone();
        let mut changed = original.clone();
        changed[0] ^= 1;
        private_file(&body_path(&corrupt, 0, 0), &changed);
        assert_eq!(error(&corrupt, limits()), DailyEvidenceError::HashMismatch);

        let truncated = one_batch();
        valid_control(&truncated);
        private_file(&body_path(&truncated, 0, 0), b"short");
        assert_eq!(
            error(&truncated, limits()),
            DailyEvidenceError::SizeMismatch
        );

        let torn = one_batch();
        valid_control(&torn);
        private_file(&metadata_path(&torn, 0), b"{\"files\":[");
        assert_eq!(
            error(&torn, limits()),
            DailyEvidenceError::InvalidBatchMetadata
        );

        let missing_metadata = one_batch();
        valid_control(&missing_metadata);
        fs::remove_file(metadata_path(&missing_metadata, 0)).unwrap();
        assert_eq!(
            error(&missing_metadata, limits()),
            DailyEvidenceError::MissingCommittedEvidence
        );
    }

    #[test]
    fn s2_t19_uncommitted_orphans_are_not_opened_or_reconciled() {
        let fixture = one_batch();
        valid_control(&fixture);
        let orphan = batch(99, "2026-10-11", &[("orphan.json", b"orphan body")]);
        write_batch(&fixture.data_root, &orphan);
        let orphan_path = batch_dir(&fixture.data_root, &orphan.entry);
        let incomplete = raw_root(&fixture.data_root)
            .join("provider=kis")
            .join("market=kr")
            .join("date=2026-10-10")
            .join("batch=incomplete");
        private_tree_dir(&fixture.data_root, &incomplete);
        let before = tree_state(fixture._temp.path());
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.batches().len(), 1);
        assert!(
            !snapshot
                .batches()
                .iter()
                .any(|batch| batch.batch_id() == orphan.entry.batch_id)
        );
        drop(snapshot);
        assert_eq!(tree_state(fixture._temp.path()), before);
        assert!(orphan_path.exists());
        assert!(incomplete.exists());
    }

    #[test]
    fn s2_t20_batch_partition_date_does_not_limit_committed_discovery() {
        let mut earlier = batch(2, "2020-01-02", &[("old.json", b"old")]);
        earlier.entry.files[0].request.query = vec![
            ("F_DT".to_owned(), "20261012".to_owned()),
            ("T_DT".to_owned(), "20261014".to_owned()),
        ];
        let fixture = fixture_with(vec![
            batch(1, "2026-10-13", &[("new.json", b"new")]),
            earlier,
        ]);
        valid_control(&fixture);
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.batches().len(), 2);
        assert_eq!(
            snapshot.batches()[0].batch_date(),
            TradingDate::parse("2026-10-13").unwrap()
        );
        assert_eq!(
            snapshot.batches()[1].batch_date(),
            TradingDate::parse("2020-01-02").unwrap()
        );
        let old_metadata: serde_json::Value =
            serde_json::from_slice(snapshot.batches()[1].batch_json_bytes()).unwrap();
        assert_eq!(
            old_metadata["files"][0]["request"]["query"],
            serde_json::json!([["F_DT", "20261012"], ["T_DT", "20261014"]])
        );
    }

    #[test]
    fn s2_t21_exact_lookup_matches_full_file_identity_without_copy_fallback() {
        let fixture = fixture();
        let snapshot = scan(&fixture, limits()).unwrap();
        let batch = &fixture.batches[0].entry;
        let file = &batch.files[0];
        let reference = entry_reference(batch, file);
        let found = snapshot.lookup_exact(&reference).unwrap();
        assert_eq!(found.bytes(), DEFAULT_BODY);
        assert_eq!(found.batch().batch_id(), batch.batch_id);

        let mut missing = reference.clone();
        missing.batch_id = BatchId::generate();
        assert_eq!(
            snapshot.lookup_exact(&missing).err(),
            Some(DailyEvidenceError::NotCommitted)
        );
        let mut wrong_hash = reference.clone();
        wrong_hash.content_hash = ContentHash::from_bytes(b"other");
        assert_eq!(
            snapshot.lookup_exact(&wrong_hash).err(),
            Some(DailyEvidenceError::ReferenceMismatch)
        );
        let mut wrong_scope = reference.clone();
        wrong_scope.provider = "another".to_owned();
        assert_eq!(
            snapshot.lookup_exact(&wrong_scope).err(),
            Some(DailyEvidenceError::ScopeMismatch)
        );
        let mut wrong_retrieval = reference.clone();
        wrong_retrieval.retrieved_at = UtcTimestamp::parse_rfc3339("2026-10-14T01:02:04Z").unwrap();
        assert_eq!(
            snapshot.lookup_exact(&wrong_retrieval).err(),
            Some(DailyEvidenceError::ReferenceMismatch)
        );
        let mut absent_copy = reference;
        absent_copy.file_name = "copied-source.json".to_owned();
        assert_eq!(
            snapshot.lookup_exact(&absent_copy).err(),
            Some(DailyEvidenceError::NotCommitted)
        );
    }

    #[test]
    fn s2_t22_two_readers_hold_shared_lock_until_both_drop() {
        let fixture = one_batch();
        let reader = CommittedRawReader::new(&fixture.data_root);
        let first = reader.snapshot(scope(&fixture), limits()).unwrap();
        let second = reader.snapshot(scope(&fixture), limits()).unwrap();
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&fixture.data_root))
            .unwrap();
        assert!(FileExt::try_lock_exclusive(&writer).is_err());
        drop(first);
        assert!(FileExt::try_lock_exclusive(&writer).is_err());
        drop(second);
        FileExt::try_lock_exclusive(&writer).unwrap();
        FileExt::unlock(&writer).unwrap();
    }

    #[test]
    fn s2_t23_exclusive_writer_is_reported_busy_without_retry() {
        let fixture = one_batch();
        valid_control(&fixture);
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&fixture.data_root))
            .unwrap();
        FileExt::lock_exclusive(&writer).unwrap();
        let competing_writer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&fixture.data_root))
            .unwrap();
        assert!(matches!(
            FileExt::try_lock_exclusive(&competing_writer),
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        drop(competing_writer);
        assert_eq!(error(&fixture, limits()), DailyEvidenceError::LockBusy);
        FileExt::unlock(&writer).unwrap();
        assert!(scan(&fixture, limits()).is_ok());
    }

    #[test]
    fn s2_t26_cooperating_append_publishes_after_snapshot_drop() {
        let fixture = one_batch();
        let next = batch(2, "2026-10-14", &[("next.json", b"next body")]);
        write_batch(&fixture.data_root, &next);
        let snapshot = scan(&fixture, limits()).unwrap();
        let data_root = fixture.data_root.clone();
        let appended = next.entry.clone();
        let lock = lock_path(&fixture.data_root);
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let probe = OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)
                .unwrap();
            let contended = match FileExt::try_lock_exclusive(&probe) {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
                Ok(()) => {
                    FileExt::unlock(&probe).unwrap();
                    false
                }
                Err(_) => false,
            };
            attempt_tx.send(contended).unwrap();
            continue_rx.recv().unwrap();
            drop(probe);
            let store = RawStore::new(data_root);
            let result = store.append_manifest("kis", "kr", &appended);
            done_tx.send(result.is_ok()).unwrap();
        });
        assert!(attempt_rx.recv().unwrap(), "writer must observe WouldBlock");
        drop(snapshot);
        continue_tx.send(()).unwrap();
        assert!(done_rx.recv().unwrap());
        worker.join().unwrap();
        let fresh = scan(&fixture, limits()).unwrap();
        assert_eq!(fresh.batches().len(), 2);
        assert_eq!(fresh.batches()[1].batch_id(), next.entry.batch_id);
    }

    #[test]
    fn s2_t27_returned_bytes_remain_captured_after_disk_mutation() {
        let fixture = one_batch();
        let snapshot = scan(&fixture, limits()).unwrap();
        let reference = entry_reference(
            &fixture.batches[0].entry,
            &fixture.batches[0].entry.files[0],
        );
        let captured = snapshot.lookup_exact(&reference).unwrap();
        let expected = captured.bytes().to_vec();
        let mut changed = expected.clone();
        changed[0] ^= 1;
        private_file(&body_path(&fixture, 0, 0), &changed);
        assert_eq!(captured.bytes(), expected.as_slice());
        drop(snapshot);
        assert_eq!(error(&fixture, limits()), DailyEvidenceError::HashMismatch);
    }

    #[test]
    fn s2_t29_fieldless_errors_never_format_raw_sentinels() {
        const SENTINEL: &str = "private-provider-path-raw-sentinel";
        let fixture = one_batch();
        valid_control(&fixture);
        rewrite_manifest(
            &fixture,
            format!("{{\"provider\":\"{SENTINEL}\"}}\n").as_bytes(),
        );
        let evidence_error = error(&fixture, limits());
        let rendered = format!(
            "{} {:?} {}",
            evidence_error,
            evidence_error,
            evidence_error.code()
        );
        assert!(!rendered.contains(SENTINEL));
        assert!(evidence_error.source().is_none());
        assert_eq!(evidence_error.to_string(), evidence_error.code());

        for field in ["batch_id", "mode", "content_hash", "file_name"] {
            let fixture = one_batch();
            valid_control(&fixture);
            let mut value = serde_json::to_value(&fixture.batches[0].entry).unwrap();
            match field {
                "batch_id" | "mode" => {
                    value.as_object_mut().unwrap().insert(
                        field.to_owned(),
                        serde_json::Value::String(SENTINEL.to_owned()),
                    );
                }
                "content_hash" | "file_name" => {
                    value["files"][0][field] = if field == "file_name" {
                        serde_json::Value::String(format!("../{SENTINEL}"))
                    } else {
                        serde_json::Value::String(SENTINEL.to_owned())
                    };
                }
                _ => unreachable!(),
            }
            rewrite_manifest(&fixture, &serde_json::to_vec(&value).unwrap());
            let evidence_error = error(&fixture, limits());
            let rendered = format!(
                "{} {:?} {}",
                evidence_error,
                evidence_error,
                evidence_error.code()
            );
            assert!(!rendered.contains(SENTINEL));
            assert!(evidence_error.source().is_none());
        }

        let metadata_fixture = one_batch();
        valid_control(&metadata_fixture);
        let mut sentinel_entry = metadata_fixture.batches[0].entry.clone();
        sentinel_entry.entitlement_reference = Some(SENTINEL.to_owned());
        sentinel_entry.files[0].request.endpoint = SENTINEL.to_owned();
        let mut copied = entry_reference(
            &metadata_fixture.batches[0].entry,
            &metadata_fixture.batches[0].entry.files[0],
        );
        copied.file_name = SENTINEL.to_owned();
        sentinel_entry.files[0].copied_from = Some(copied);
        rewrite_manifest(
            &metadata_fixture,
            &serde_json::to_vec(&sentinel_entry).unwrap(),
        );
        rewrite_batch_metadata(&metadata_fixture, 0, &sentinel_entry);
        let mut changed_body = metadata_fixture.batches[0].bodies[0].1.clone();
        changed_body[0] ^= 1;
        private_file(&body_path(&metadata_fixture, 0, 0), &changed_body);
        let evidence_error = error(&metadata_fixture, limits());
        let rendered = format!(
            "{} {:?} {}",
            evidence_error,
            evidence_error,
            evidence_error.code()
        );
        assert!(!rendered.contains(SENTINEL));
        assert!(evidence_error.source().is_none());

        let os_fixture = one_batch();
        valid_control(&os_fixture);
        let missing = os_fixture._temp.path().join(SENTINEL).join("data");
        let evidence_error = CommittedRawReader::new(&missing)
            .snapshot(scope(&os_fixture), limits())
            .err()
            .unwrap();
        let rendered = format!(
            "{} {:?} {}",
            evidence_error,
            evidence_error,
            evidence_error.code()
        );
        assert!(!rendered.contains(SENTINEL));
        assert!(evidence_error.source().is_none());
    }

    #[test]
    fn s2_t30_unknown_fields_optional_raw_fields_and_invalid_provider_body_stay_opaque() {
        let fixture = one_batch();
        valid_control(&fixture);
        let mut value = serde_json::to_value(&fixture.batches[0].entry).unwrap();
        value.as_object_mut().unwrap().insert(
            "legacy_unknown_field".to_owned(),
            serde_json::json!({"nested": ["ignored"]}),
        );
        let files = value
            .as_object_mut()
            .unwrap()
            .get_mut("files")
            .unwrap()
            .as_array_mut()
            .unwrap();
        files[0]
            .as_object_mut()
            .unwrap()
            .remove("response_continuation");
        files[0].as_object_mut().unwrap().remove("copied_from");
        let raw = serde_json::to_vec(&value).unwrap();
        rewrite_manifest(&fixture, &raw);
        let snapshot = scan(&fixture, limits()).unwrap();
        assert_eq!(snapshot.batches().len(), 1);
        assert_eq!(
            snapshot.batches()[0].files().next().unwrap().bytes(),
            DEFAULT_BODY
        );
    }

    #[test]
    fn s2_t32_explicit_fixture_uid_and_writable_outside_ancestor_policy_are_preserved() {
        let fixture = one_batch();
        valid_control(&fixture);
        set_mode(&fixture.data_root, 0o777);
        let explicit_uid = fs::metadata(raw_root(&fixture.data_root)).unwrap().uid();
        let explicit_scope = ApprovedRawScope::new("kis", "kr", explicit_uid).unwrap();
        assert_eq!(explicit_scope.trusted_raw_owner_uid(), explicit_uid);
        let snapshot = CommittedRawReader::new(&fixture.data_root)
            .snapshot(explicit_scope, limits())
            .unwrap();
        assert_eq!(snapshot.batches().len(), 1);
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn s2_t31_non_linux_returns_unsupported_without_path_io() {
    use market_data::daily_price_only::source::{
        ApprovedRawScope, CommittedRawReader, DailyEvidenceError, ScanLimitValues, ScanLimits,
    };
    use std::path::Path;

    let scope = ApprovedRawScope::new("kis", "kr", 4242).unwrap();
    let limits = ScanLimits::new(ScanLimitValues {
        max_manifest_bytes: 16,
        max_manifest_records: 1,
        max_batches: 1,
        max_files_per_batch: 1,
        max_files: 1,
        max_batch_metadata_bytes: 16,
        max_source_file_bytes: 16,
        max_retained_bytes: 64,
        max_open_descriptors: 16,
        max_data_root_bytes: 128,
        max_data_root_components: 8,
        max_metadata_json_depth: 32,
    })
    .unwrap();
    assert_eq!(
        CommittedRawReader::new(Path::new("relative/nonexistent"))
            .snapshot(scope, limits)
            .err(),
        Some(DailyEvidenceError::UnsupportedPlatform)
    );
}
