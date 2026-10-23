//! Offline I1b-S3 lineage/calendar fixtures. No live provider or DB authority.

#[cfg(target_os = "linux")]
mod linux_tests {
    use std::error::Error;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
    use fs2::FileExt;
    use market_data::contract::{FetchMode, RequestMetadata, ResponseKind, SourceFileReference};
    use market_data::daily_price_only::DailySessionTypeV1;
    use market_data::daily_price_only::calendar::{
        CalendarEvidenceKind, VerifiedCalendarProjection, eligible_calendar_date,
        expected_calendar_session, project_calendar, prove_calendar_range,
    };
    use market_data::daily_price_only::source::{
        ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, DailyEvidenceError,
        ScanLimitValues, ScanLimits,
    };
    use market_data::daily_price_only::verify::{
        DailyVerificationError as E, RawEvidenceSnapshots, ResolvedOriginal, resolve_original,
    };
    use market_data::storage::{FileEntry, ManifestEntry};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use uuid::Uuid;

    const ENTITLEMENT: &str = "fixture:read-only-licensed";
    const BODY: &[u8] = br#"{"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},{"bass_dt":"20261010","opnd_yn":"N"},{"bass_dt":"20261011","opnd_yn":"N"},{"bass_dt":"20261012","opnd_yn":"Y"}]}"#;

    fn date(s: &str) -> TradingDate {
        TradingDate::parse(s).unwrap()
    }
    fn time(s: &str) -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339(s).unwrap()
    }
    fn observed() -> UtcTimestamp {
        time("2026-10-12T07:30:00Z")
    }
    fn owner() -> Uuid {
        Uuid::from_u128(999)
    }
    fn policy() -> ContentHash {
        ContentHash::from_bytes(b"fixture-policy")
    }

    struct Batch {
        entry: ManifestEntry,
        bytes: Vec<u8>,
    }

    impl Batch {
        fn reference(&self) -> SourceFileReference {
            SourceFileReference {
                provider: self.entry.provider.clone(),
                market: self.entry.market.clone(),
                batch_id: self.entry.batch_id,
                file_name: self.entry.files[0].file_name.clone(),
                content_hash: self.entry.files[0].content_hash.clone(),
                retrieved_at: self.entry.retrieved_at,
            }
        }
    }

    fn batch(id: u128, provider: &str, body: &[u8]) -> Batch {
        Batch {
            entry: ManifestEntry {
                batch_id: BatchId::from_uuid(Uuid::from_u128(id)),
                provider: provider.to_owned(),
                market: "kr".to_owned(),
                date: date("2026-10-09"),
                retrieved_at: time("2026-10-09T07:30:00Z"),
                mode: FetchMode::Credentialed,
                entitlement_reference: Some(ENTITLEMENT.to_owned()),
                files: vec![FileEntry {
                    kind: ResponseKind::Calendar,
                    file_name: "calendar.json".to_owned(),
                    content_hash: ContentHash::from_bytes(body),
                    size_bytes: body.len() as u64,
                    request: RequestMetadata {
                        endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".to_owned(),
                        query: vec![
                            ("BASS_DT".to_owned(), "20261009".to_owned()),
                            ("CTX_AREA_FK".to_owned(), String::new()),
                            ("CTX_AREA_NK".to_owned(), String::new()),
                        ],
                        headers: vec![
                            ("appkey".to_owned(), "[REDACTED]".to_owned()),
                            ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                            ("authorization".to_owned(), "[REDACTED]".to_owned()),
                            ("tr_cont".to_owned(), String::new()),
                            ("tr_id".to_owned(), "CTCA0903R".to_owned()),
                        ],
                        mode: FetchMode::Credentialed,
                    },
                    response_continuation: Some("M".to_owned()),
                    copied_from: None,
                }],
            },
            bytes: body.to_vec(),
        }
    }

    fn copy(id: u128, original: &Batch) -> Batch {
        let mut copied = batch(id, "kis", &original.bytes);
        copied.entry.date = date("2026-10-12");
        copied.entry.retrieved_at = time("2026-10-12T01:00:00Z");
        copied.entry.files[0].request = original.entry.files[0].request.clone();
        copied.entry.files[0].copied_from = Some(original.reference());
        copied
    }

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        uid: u32,
        files: Vec<PathBuf>,
    }

    fn private_dir(root: &Path, path: &Path) {
        let mut current = root.to_path_buf();
        for component in path.strip_prefix(root).unwrap().components() {
            current.push(component);
            if !current.exists() {
                fs::create_dir(&current).unwrap();
                fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
    }

    impl Fixture {
        fn new(batches: &[Batch]) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("data");
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
            let mut files = Vec::new();
            let mut write = |path: PathBuf, bytes: &[u8]| {
                private_dir(&root, path.parent().unwrap());
                fs::write(&path, bytes).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                files.push(path);
            };
            for provider in ["kis", "kis-calendar"] {
                let manifest = root.join(format!("raw/manifests/provider={provider}/market=kr"));
                write(manifest.join("commit.lock"), b"");
                let mut lines = Vec::new();
                for batch in batches.iter().filter(|b| b.entry.provider == provider) {
                    let dir = root.join(format!(
                        "raw/provider={provider}/market=kr/date={}/batch={}",
                        batch.entry.date, batch.entry.batch_id
                    ));
                    write(
                        dir.join("batch.json"),
                        &serde_json::to_vec_pretty(&batch.entry).unwrap(),
                    );
                    write(dir.join(&batch.entry.files[0].file_name), &batch.bytes);
                    lines.extend(serde_json::to_vec(&batch.entry).unwrap());
                    lines.push(b'\n');
                }
                write(manifest.join("manifest.jsonl"), &lines);
            }
            let uid = fs::metadata(root.join("raw")).unwrap().uid();
            Self {
                _temp: temp,
                root,
                uid,
                files,
            }
        }

        fn scan(&self, provider: &str) -> CommittedRawSnapshot {
            let values = ScanLimitValues {
                max_manifest_bytes: 262_144,
                max_manifest_records: 64,
                max_batches: 64,
                max_files_per_batch: 8,
                max_files: 64,
                max_batch_metadata_bytes: 16_384,
                max_source_file_bytes: 1_048_576,
                max_retained_bytes: 8_388_608,
                max_open_descriptors: 512,
                max_data_root_bytes: 4096,
                max_data_root_components: 32,
                max_metadata_json_depth: 32,
            };
            CommittedRawReader::new(&self.root)
                .snapshot(
                    ApprovedRawScope::new(provider, "kr", self.uid).unwrap(),
                    ScanLimits::new(values).unwrap(),
                )
                .unwrap()
        }

        fn lock(&self, provider: &str) -> fs::File {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.root.join(format!(
                    "raw/manifests/provider={provider}/market=kr/commit.lock"
                )))
                .unwrap()
        }

        fn fingerprint(&self) -> Vec<(ContentHash, u32, i64, i64, u64)> {
            self.files
                .iter()
                .map(|file| {
                    let metadata = fs::metadata(file).unwrap();
                    (
                        ContentHash::from_bytes(&fs::read(file).unwrap()),
                        metadata.mode(),
                        metadata.mtime(),
                        metadata.mtime_nsec(),
                        metadata.ino(),
                    )
                })
                .collect()
        }
    }

    fn resolve<'s>(
        scopes: &[&'s CommittedRawSnapshot],
        reference: &SourceFileReference,
    ) -> Result<ResolvedOriginal<'s>, E> {
        resolve_original(
            &RawEvidenceSnapshots::new(scopes, ENTITLEMENT, observed()).unwrap(),
            reference,
        )
    }

    fn project<'s>(
        scopes: &[&'s CommittedRawSnapshot],
        reference: &SourceFileReference,
    ) -> Result<VerifiedCalendarProjection<'s>, E> {
        project_calendar(&resolve(scopes, reference)?, owner(), &policy())
    }

    fn projection_error(body: &[u8]) -> E {
        let original = batch(1, "kis-calendar", body);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        project(&[&snapshot], &reference).err().unwrap()
    }

    #[test]
    fn s3_cross_scope_copy_retains_original_date_precision_and_all_commitments() {
        let original = batch(1, "kis-calendar", BODY);
        let copied = copy(2, &original);
        let reference = copied.reference();
        let fixture = Fixture::new(&[original, copied]);
        let a = fixture.scan("kis");
        let b = fixture.scan("kis-calendar");
        let resolved = resolve(&[&a, &b], &reference).unwrap();
        let evidence = resolved.evidence();
        assert_eq!(evidence.provider, "kis");
        assert_eq!(evidence.batch_date, date("2026-10-12"));
        assert_eq!(evidence.copied_from.len(), 1);
        let origin = &evidence.copied_from[0];
        assert_eq!(origin.provider, "kis-calendar");
        assert_eq!(origin.batch_date, date("2026-10-09"));
        assert_eq!(
            evidence.original_retrieved_at.as_timestamp(),
            time("2026-10-09T07:30:00Z")
        );
        assert_eq!(evidence.original_retrieval_precision, "batch");
        assert_eq!(evidence.retrieval_precision, "batch");
        assert_eq!(origin.retrieval_precision, "batch");
        assert_eq!(resolved.original_bytes(), BODY);
        for (scope, hash, line_hash) in [
            (
                &a,
                &evidence.batch_json_sha256,
                &evidence.manifest_line_sha256,
            ),
            (&b, &origin.batch_json_sha256, &origin.manifest_line_sha256),
        ] {
            let batch = scope.batches().first().unwrap();
            assert_eq!(*hash, ContentHash::from_bytes(batch.batch_json_bytes()));
            assert_eq!(
                *line_hash,
                ContentHash::from_bytes(batch.manifest_line_bytes())
            );
        }
        let projected = project_calendar(&resolved, owner(), &policy()).unwrap();
        assert_eq!(projected.days().len(), 4);
        assert_eq!(
            projected.days()[0].evidence_kind(),
            CalendarEvidenceKind::RequestedTarget
        );
        assert_eq!(
            projected.days()[3].evidence_kind(),
            CalendarEvidenceKind::DerivedSurrounding
        );
        assert_eq!(
            projected.source_file().response_continuation.as_deref(),
            Some("M")
        );
    }

    #[test]
    fn s3_exact_lookup_rejects_missing_original_and_reference_hash_or_time() {
        let original = batch(1, "kis-calendar", BODY);
        let copied = copy(2, &original);
        let reference = copied.reference();
        let fixture = Fixture::new(&[copied]);
        let a = fixture.scan("kis");
        let b = fixture.scan("kis-calendar");
        assert_eq!(
            resolve(&[&a, &b], &reference).err(),
            Some(E::Snapshot(DailyEvidenceError::NotCommitted))
        );
        for change in 0..2 {
            let mut wrong = reference.clone();
            if change == 0 {
                wrong.content_hash = ContentHash::from_bytes(b"wrong");
            } else {
                wrong.retrieved_at = observed();
            }
            assert_eq!(
                resolve(&[&a], &wrong).err(),
                Some(E::Snapshot(DailyEvidenceError::ReferenceMismatch))
            );
        }
        assert_eq!(
            resolve(&[&a], &reference).err(),
            Some(E::Snapshot(DailyEvidenceError::ScopeMismatch))
        );
    }

    #[test]
    fn s3_cycle_and_sixteen_link_boundary_are_distinct() {
        let mut original = batch(1, "kis-calendar", BODY);
        original.entry.files[0].copied_from = Some(original.reference());
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        assert_eq!(resolve(&[&snapshot], &reference).err(), Some(E::CopyCycle));
        for links in [16usize, 17] {
            let mut batches = vec![batch(1, "kis-calendar", BODY)];
            for id in 2..=links + 1 {
                batches.push(copy(id as u128, batches.last().unwrap()));
            }
            let reference = batches.last().unwrap().reference();
            let fixture = Fixture::new(&batches);
            let a = fixture.scan("kis");
            let b = fixture.scan("kis-calendar");
            let result = resolve(&[&a, &b], &reference);
            if links == 16 {
                assert_eq!(result.unwrap().evidence().copied_from.len(), 16);
            } else {
                assert_eq!(result.err(), Some(E::CopyDepth));
            }
        }
    }

    #[test]
    fn s3_every_hop_requires_credentialed_mode_and_expected_entitlement() {
        for location in 0..2 {
            for change in 0..3 {
                let original = batch(1, "kis-calendar", BODY);
                let copied = copy(2, &original);
                let mut batches = [original, copied];
                match change {
                    0 => batches[location].entry.mode = FetchMode::Synthetic,
                    1 => batches[location].entry.files[0].request.mode = FetchMode::Synthetic,
                    _ => batches[location].entry.entitlement_reference = Some("other".to_owned()),
                }
                let reference = batches[1].reference();
                let fixture = Fixture::new(&batches);
                let a = fixture.scan("kis");
                let b = fixture.scan("kis-calendar");
                assert_eq!(
                    resolve(&[&a, &b], &reference).err(),
                    Some(if change == 2 {
                        E::EntitlementMismatch
                    } else {
                        E::NonCredentialed
                    })
                );
            }
        }
    }

    #[test]
    fn s3_copy_kind_request_continuation_and_bytes_must_match() {
        for change in 0..4 {
            let original = batch(1, "kis-calendar", BODY);
            let mut copied = copy(2, &original);
            match change {
                0 => copied.entry.files[0].kind = ResponseKind::Bars,
                1 => copied.entry.files[0].request.query[0].1 = "20261012".to_owned(),
                2 => copied.entry.files[0].response_continuation = None,
                _ => {
                    copied.bytes.push(b' ');
                    copied.entry.files[0].size_bytes += 1;
                    copied.entry.files[0].content_hash = ContentHash::from_bytes(&copied.bytes);
                }
            }
            let reference = copied.reference();
            let fixture = Fixture::new(&[original, copied]);
            let a = fixture.scan("kis");
            let b = fixture.scan("kis-calendar");
            assert_eq!(resolve(&[&a, &b], &reference).err(), Some(E::CopyMismatch));
        }
    }

    #[test]
    fn s3_time_order_and_future_retrieval_cannot_be_reset_by_copy() {
        let original = batch(1, "kis-calendar", BODY);
        let mut copied = copy(2, &original);
        copied.entry.retrieved_at = time("2026-10-08T07:30:00Z");
        let reference = copied.reference();
        let fixture = Fixture::new(&[original, copied]);
        let a = fixture.scan("kis");
        let b = fixture.scan("kis-calendar");
        assert_eq!(resolve(&[&a, &b], &reference).err(), Some(E::CopyMismatch));
        let original = batch(1, "kis-calendar", BODY);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-09T07:29:59Z"))
                .unwrap();
        assert_eq!(
            resolve_original(&context, &reference).err(),
            Some(E::FutureRetrieval)
        );
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-09T07:30:00Z"))
                .unwrap();
        assert!(resolve_original(&context, &reference).is_ok());
    }

    #[test]
    fn s3_context_rejects_empty_duplicate_scopes_and_empty_entitlement() {
        let fixture = Fixture::new(&[]);
        let snapshot = fixture.scan("kis");
        for scopes in [vec![], vec![&snapshot, &snapshot]] {
            assert_eq!(
                RawEvidenceSnapshots::new(&scopes, ENTITLEMENT, observed()).err(),
                Some(E::InvalidContext)
            );
        }
        assert_eq!(
            RawEvidenceSnapshots::new(&[&snapshot], " ", observed()).err(),
            Some(E::InvalidContext)
        );
    }

    #[test]
    fn s3_first_page_request_is_exact_and_response_marker_is_not_followed() {
        for change in 0..5 {
            let mut original = batch(1, "kis-calendar", BODY);
            match change {
                0 => original.entry.files[0].request.query[0].1 = "20261010".to_owned(),
                1 => original.entry.files[0].request.headers[3].1 = "N".to_owned(),
                2 => original.entry.files[0].request.headers[0].1 = "not-redacted".to_owned(),
                3 => original.entry.files[0]
                    .request
                    .query
                    .push(("BASS_DT".to_owned(), "20261009".to_owned())),
                _ => original.entry.files[0].request.headers[4].1 = "WRONG".to_owned(),
            }
            let reference = original.reference();
            let fixture = Fixture::new(&[original]);
            let snapshot = fixture.scan("kis-calendar");
            assert_eq!(
                project(&[&snapshot], &reference).err(),
                Some(E::InvalidEvidence)
            );
        }
        let mut original = batch(1, "kis-calendar", BODY);
        original.entry.files[0].request.headers.reverse();
        original.entry.files[0].request.query.reverse();
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        assert!(project(&[&snapshot], &reference).is_ok());
    }

    #[test]
    fn s3_original_target_is_required_even_with_valid_surrounding_rows() {
        assert_eq!(
            projection_error(br#"{"rt_cd":"0","output":[{"bass_dt":"20261012","opnd_yn":"Y"}]}"#),
            E::CalendarTargetMissing
        );
    }

    #[test]
    fn s3_every_surrounding_row_and_duplicate_date_are_checked() {
        for row in [
            json!({"bass_dt":"20261010","opnd_yn":"bad"}),
            json!({"bass_dt":"nonsense","opnd_yn":"Y"}),
            json!({"bass_dt":"20261010","opnd_yn":true}),
            json!({"opnd_yn":"Y"}),
            json!(null),
        ] {
            let body = serde_json::to_vec(
                &json!({"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},row]}),
            )
            .unwrap();
            assert_eq!(projection_error(&body), E::InvalidCalendar);
        }
        for flag in ["Y", "N"] {
            let body = serde_json::to_vec(&json!({"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},{"bass_dt":"20261010","opnd_yn":"N"},{"bass_dt":"20261010","opnd_yn":flag}]})).unwrap();
            assert_eq!(projection_error(&body), E::CalendarConflict);
        }
    }

    #[test]
    fn s3_duplicate_json_keys_escaped_aliases_trailing_and_depth_fail_closed() {
        for body in [
            br#"{"rt_cd":"0","rt_cd":"0","output":{}}"#.as_slice(),
            br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"Y","\u006fpnd_yn":"N"}}"#,
            br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"Y","bass_dt":[}}"#,
            br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"Y","x":{"a":1,"\u0061":2}}}"#,
            br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"Y"}} {}"#,
        ] { assert_eq!(projection_error(body), E::InvalidProviderJson); }
        for nesting in [30, 31] {
            let mut body =
                br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"Y","x":"#.to_vec();
            body.extend(std::iter::repeat_n(b'[', nesting));
            body.extend_from_slice(b"0");
            body.extend(std::iter::repeat_n(b']', nesting));
            body.extend_from_slice(b"}}");
            if nesting == 31 {
                assert_eq!(projection_error(&body), E::InvalidProviderJson);
            } else {
                let original = batch(1, "kis-calendar", &body);
                let reference = original.reference();
                let fixture = Fixture::new(&[original]);
                let snapshot = fixture.scan("kis-calendar");
                assert!(project(&[&snapshot], &reference).is_ok());
            }
        }
    }

    #[test]
    fn s3_provider_errors_are_fixed_and_do_not_expose_free_form_content() {
        for rt in [json!("1"), json!(0), json!(null)] {
            let body =
                serde_json::to_vec(&json!({"rt_cd":rt,"msg1":"SECRET_SENTINEL","output":[]}))
                    .unwrap();
            let error = projection_error(&body);
            assert_eq!(error, E::ProviderResponse);
            assert_eq!(error.to_string(), "PROVIDER_RESPONSE");
            assert!(error.source().is_none());
            assert!(!format!("{error:?}").contains("SECRET_SENTINEL"));
        }
    }

    #[test]
    fn s3_object_output_has_ordinal_zero_and_future_rows_are_allowed() {
        let body = br#"{"rt_cd":"0","output":{"opnd_yn":"Y","bass_dt":"20261009"}}"#;
        let original = batch(1, "kis-calendar", body);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let projection = project(&[&snapshot], &reference).unwrap();
        assert_eq!(projection.days()[0].source_row_ordinal(), 0);
        assert_eq!(
            projection.days()[0].source_row_sha256(),
            &ContentHash::from_bytes(br#"{"bass_dt":"20261009","opnd_yn":"Y"}"#)
        );
        let original = batch(1, "kis-calendar", BODY);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-09T07:30:00Z"))
                .unwrap();
        let resolved = resolve_original(&context, &reference).unwrap();
        let projection = project_calendar(&resolved, owner(), &policy()).unwrap();
        assert_eq!(projection.days()[3].session_date(), date("2026-10-12"));
    }

    #[test]
    fn s3_projection_limits_cover_count_span_and_exact_370_day_boundary() {
        for count in [0, 370, 371] {
            let rows: Vec<Value> = (0..count).map(|n| json!({"bass_dt":date("2026-10-09").checked_add_days(n).unwrap().to_iso().replace('-', ""),"opnd_yn":"N"})).collect();
            let body = serde_json::to_vec(&json!({"rt_cd":"0","output":rows})).unwrap();
            let original = batch(1, "kis-calendar", &body);
            let reference = original.reference();
            let fixture = Fixture::new(&[original]);
            let snapshot = fixture.scan("kis-calendar");
            let result = project(&[&snapshot], &reference);
            if count == 370 {
                assert_eq!(result.unwrap().days().len(), 370);
            } else {
                assert_eq!(result.err(), Some(E::LimitExceeded));
            }
        }
        assert_eq!(projection_error(br#"{"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},{"bass_dt":"20271014","opnd_yn":"Y"}]}"#), E::LimitExceeded);
    }

    #[test]
    fn s3_row_hash_sorts_nested_objects_but_preserves_array_order_and_ordinal() {
        let bodies: [&[u8]; 3] = [
            br#"{"rt_cd":"0","output":[{"bass_dt":"20261010","opnd_yn":"N"},{"x":{"z":-2,"a":[2,1]},"opnd_yn":"Y","bass_dt":"20261009"}]}"#,
            br#"{"output":[{"opnd_yn":"N","bass_dt":"20261010"},{"bass_dt":"20261009","opnd_yn":"Y","x":{"a":[2,1],"z":-2}}],"rt_cd":"0"}"#,
            br#"{"rt_cd":"0","output":[{"bass_dt":"20261010","opnd_yn":"N"},{"bass_dt":"20261009","opnd_yn":"Y","x":{"a":[1,2],"z":-2}}]}"#,
        ];
        let mut hashes = Vec::new();
        let mut sources = Vec::new();
        for body in bodies {
            let original = batch(1, "kis-calendar", body);
            let reference = original.reference();
            let fixture = Fixture::new(&[original]);
            let snapshot = fixture.scan("kis-calendar");
            let projection = project(&[&snapshot], &reference).unwrap();
            let target = &projection.days()[0];
            assert_eq!(target.source_row_ordinal(), 1);
            hashes.push(target.source_row_sha256().clone());
            sources.push(projection.evidence_sha256().clone());
        }
        assert_eq!(
            hashes[0],
            ContentHash::from_bytes(
                br#"{"bass_dt":"20261009","opnd_yn":"Y","x":{"a":[2,1],"z":-2}}"#
            )
        );
        assert_eq!(hashes[0], hashes[1]);
        assert_ne!(hashes[0], hashes[2]);
        assert_ne!(sources[0], sources[1]);
    }

    #[test]
    fn s3_projection_identity_and_reconstructed_document_have_exact_golden_bytes() {
        let original = batch(1, "kis-calendar", BODY);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let resolved = resolve(&[&snapshot], &reference).unwrap();
        let projection = project_calendar(&resolved, owner(), &policy()).unwrap();
        let mut preimage = format!(
            "owner-beta-daily-calendar-source-v1\n{}\n{}\ndaily-raw-reconstruction-v1\n",
            owner(),
            policy()
        )
        .into_bytes();
        preimage.extend(serde_json::to_vec(resolved.evidence()).unwrap());
        assert_eq!(
            projection.evidence_sha256(),
            &ContentHash::from_bytes(&preimage)
        );
        let document = projection.document(date("2026-10-12")).unwrap();
        let source_preimage = format!(
            "{{\"schema\":\"owner-beta-daily-normalized-source-v1\",\"kind\":\"calendar\",\"session_date\":\"2026-10-12\",\"source_files\":[{{\"provider\":\"kis-calendar\",\"market\":\"kr\",\"batch_id\":\"00000000-0000-0000-0000-000000000001\",\"file_name\":\"calendar.json\",\"wire_sha256\":\"{}\"}}]}}",
            ContentHash::from_bytes(BODY)
        );
        assert_eq!(
            document.descriptor().source_evidence_sha256,
            ContentHash::from_bytes(source_preimage.as_bytes())
        );
        let expected = format!(
            "{{\"kind\":\"calendar\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[{{\"date\":\"2026-10-12\",\"evidence_kind\":\"DERIVED_SURROUNDING\",\"session_type\":\"TRADING\"}}],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"2026-10-12\",\"source_evidence_sha256\":\"{}\"}}",
            document.descriptor().source_evidence_sha256
        );
        assert_eq!(document.bytes(), expected.as_bytes());
        assert_eq!(
            document.descriptor().sha256,
            ContentHash::from_bytes(expected.as_bytes())
        );
        assert_eq!(document.descriptor().row_count, 1);
        assert_eq!(document.descriptor().size_bytes, expected.len() as u64);
        assert_eq!(
            projection.document(date("2026-10-12")).unwrap().bytes(),
            document.bytes()
        );
        let days = serde_json::to_value(projection.days()).unwrap();
        assert!(
            days.as_array()
                .unwrap()
                .iter()
                .all(|day| day.as_object().unwrap().len() == 5)
        );
        assert_eq!(
            projection.document(date("2026-10-13")).err().unwrap(),
            E::CalendarCoverageGap
        );
        assert_eq!(
            project_calendar(&resolved, Uuid::nil(), &policy())
                .err()
                .unwrap(),
            E::InvalidIdentity
        );
        assert_ne!(
            project_calendar(&resolved, Uuid::from_u128(1000), &policy())
                .unwrap()
                .evidence_sha256(),
            projection.evidence_sha256()
        );
        assert_ne!(
            project_calendar(&resolved, owner(), &ContentHash::from_bytes(b"different"))
                .unwrap()
                .evidence_sha256(),
            projection.evidence_sha256()
        );
    }

    #[test]
    fn s3_continuous_proof_preserves_equivalent_witnesses_and_deterministic_choice() {
        let original = batch(1, "kis-calendar", BODY);
        let copied = copy(2, &original);
        let a_ref = original.reference();
        let b_ref = copied.reference();
        let fixture = Fixture::new(&[original, copied]);
        let a = fixture.scan("kis");
        let b = fixture.scan("kis-calendar");
        let p = project(&[&a, &b], &a_ref).unwrap();
        let q = project(&[&a, &b], &b_ref).unwrap();
        let proof =
            prove_calendar_range(&[&q, &p, &p], date("2026-10-09"), date("2026-10-12")).unwrap();
        let reversed =
            prove_calendar_range(&[&p, &q], date("2026-10-09"), date("2026-10-12")).unwrap();
        assert_eq!(proof.days(), reversed.days());
        assert_eq!(proof.days().len(), 4);
        assert_eq!(proof.owner(), owner());
        assert_eq!(proof.policy_sha256(), &policy());
        for day in proof.days() {
            assert_eq!(day.witnesses().len(), 2);
            assert_eq!(
                day.selected_evidence().as_str(),
                p.evidence_sha256()
                    .as_str()
                    .min(q.evidence_sha256().as_str())
            );
        }
        let excessive_claims = vec![&p; 65_537];
        assert_eq!(
            prove_calendar_range(&excessive_claims, date("2026-10-09"), date("2026-10-09")).err(),
            Some(E::LimitExceeded)
        );
    }

    #[test]
    fn s3_conflicting_overlap_blocks_entire_proof_even_outside_requested_range() {
        let original = batch(1, "kis-calendar", BODY);
        let conflict = batch(2, "kis-calendar", br#"{"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},{"bass_dt":"20261012","opnd_yn":"N"}]}"#);
        let a_ref = original.reference();
        let b_ref = conflict.reference();
        let fixture = Fixture::new(&[original, conflict]);
        let snapshot = fixture.scan("kis-calendar");
        let p = project(&[&snapshot], &a_ref).unwrap();
        let q = project(&[&snapshot], &b_ref).unwrap();
        assert_eq!(
            prove_calendar_range(&[&p, &q], date("2026-10-09"), date("2026-10-09")).err(),
            Some(E::CalendarConflict)
        );
    }

    #[test]
    fn s3_weekend_gap_never_becomes_a_weekday_guess() {
        let original = batch(1, "kis-calendar", br#"{"rt_cd":"0","output":[{"bass_dt":"20261009","opnd_yn":"Y"},{"bass_dt":"20261012","opnd_yn":"Y"}]}"#);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let p = project(&[&snapshot], &reference).unwrap();
        assert_eq!(
            prove_calendar_range(&[&p], date("2026-10-09"), date("2026-10-12")).err(),
            Some(E::CalendarCoverageGap)
        );
        assert_eq!(
            expected_calendar_session(&[&p], date("2026-10-09"), observed()).err(),
            Some(E::CalendarCoverageGap)
        );
    }

    #[test]
    fn s3_expected_session_respects_exact_kst_cutoff_and_future_observation() {
        assert_eq!(
            eligible_calendar_date(time("2026-10-12T07:29:59.999999999Z")).unwrap(),
            date("2026-10-11")
        );
        assert_eq!(
            eligible_calendar_date(time("2026-10-12T07:30:00Z")).unwrap(),
            date("2026-10-12")
        );
        let original = batch(1, "kis-calendar", BODY);
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-09T07:30:00Z"))
                .unwrap();
        let r = resolve_original(&context, &reference).unwrap();
        let p = project_calendar(&r, owner(), &policy()).unwrap();
        let before =
            expected_calendar_session(&[&p], date("2026-10-09"), time("2026-10-12T07:29:59Z"))
                .unwrap();
        assert_eq!(before.session(), date("2026-10-09"));
        assert_eq!(before.eligible_date(), date("2026-10-11"));
        assert_eq!(before.proof().days().len(), 3);
        assert_eq!(
            expected_calendar_session(&[&p], date("2026-10-09"), observed())
                .unwrap()
                .session(),
            date("2026-10-12")
        );
        let later = project(&[&snapshot], &reference).unwrap();
        assert_eq!(
            expected_calendar_session(&[&later], date("2026-10-09"), time("2026-10-12T07:29:59Z"))
                .err(),
            Some(E::FutureRetrieval)
        );
    }

    #[test]
    fn s3_proof_identity_bounds_and_no_trading_are_explicit() {
        let original = batch(
            1,
            "kis-calendar",
            br#"{"rt_cd":"0","output":{"bass_dt":"20261009","opnd_yn":"N"}}"#,
        );
        let reference = original.reference();
        let fixture = Fixture::new(&[original]);
        let snapshot = fixture.scan("kis-calendar");
        let r = resolve(&[&snapshot], &reference).unwrap();
        let p = project_calendar(&r, owner(), &policy()).unwrap();
        let q = project_calendar(&r, Uuid::from_u128(1000), &policy()).unwrap();
        assert_eq!(
            prove_calendar_range(&[&p, &q], date("2026-10-09"), date("2026-10-09")).err(),
            Some(E::InvalidIdentity)
        );
        assert_eq!(
            prove_calendar_range(&[], date("2026-10-09"), date("2026-10-09")).err(),
            Some(E::InvalidContext)
        );
        assert_eq!(
            prove_calendar_range(&[&p], date("2026-10-10"), date("2026-10-09")).err(),
            Some(E::LimitExceeded)
        );
        assert_eq!(
            prove_calendar_range(&[&p], date("2026-10-09"), date("2027-10-14")).err(),
            Some(E::LimitExceeded)
        );
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-09T07:30:00Z"))
                .unwrap();
        let r = resolve_original(&context, &reference).unwrap();
        let p = project_calendar(&r, owner(), &policy()).unwrap();
        assert_eq!(
            expected_calendar_session(&[&p], date("2026-10-09"), time("2026-10-09T07:30:00Z"))
                .err(),
            Some(E::NoTradingSession)
        );
        assert_eq!(p.days()[0].session_type(), DailySessionTypeV1::Closed);
    }

    #[test]
    fn s3_reconstruction_is_read_only_and_both_existing_locks_remain_held() {
        let original = batch(1, "kis-calendar", BODY);
        let copied = copy(2, &original);
        let reference = copied.reference();
        let fixture = Fixture::new(&[original, copied]);
        let before = fixture.fingerprint();
        {
            let a = fixture.scan("kis");
            let b = fixture.scan("kis-calendar");
            let p = project(&[&a, &b], &reference).unwrap();
            let _doc = p.document(date("2026-10-12")).unwrap();
            let _proof =
                prove_calendar_range(&[&p], date("2026-10-09"), date("2026-10-12")).unwrap();
            for provider in ["kis", "kis-calendar"] {
                assert_eq!(
                    fixture
                        .lock(provider)
                        .try_lock_exclusive()
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::WouldBlock
                );
            }
            assert_eq!(fixture.fingerprint(), before);
        }
        for provider in ["kis", "kis-calendar"] {
            let lock = fixture.lock(provider);
            lock.try_lock_exclusive().unwrap();
            FileExt::unlock(&lock).unwrap();
        }
        assert_eq!(fixture.fingerprint(), before);
    }
}
