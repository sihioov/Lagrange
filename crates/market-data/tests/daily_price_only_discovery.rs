//! Complete synthetic snapshot discovery; no live collection or admission.
#[cfg(target_os = "linux")]
mod linux_tests {
    use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
    use fs2::FileExt;
    use market_data::contract::{FetchMode, RequestMetadata, ResponseKind, SourceFileReference};
    use market_data::daily_price_only::discovery::{
        DiscoveredSessionSources, SessionSourceRequest, discover_session_sources,
    };
    use market_data::daily_price_only::observation::{DailyBlockCode, compare_source_observation};
    use market_data::daily_price_only::source::{
        ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, ScanLimitValues, ScanLimits,
    };
    use market_data::daily_price_only::verify::{
        DailyVerificationError as E, RawEvidenceSnapshots,
    };
    use market_data::daily_price_only::{
        DailySessionEvidenceV1Wire, DailySessionTypeV1, daily_price_evidence_preimage_v1,
    };
    use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
    use market_data::storage::{FileEntry, ManifestEntry};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;
    use uuid::Uuid;
    const ENTITLEMENT: &str = "fixture:licensed";
    const DAY: &str = "2026-10-12";

    fn date(text: &str) -> TradingDate {
        TradingDate::parse(text).unwrap()
    }
    fn time(text: &str) -> UtcTimestamp {
        UtcTimestamp::parse_rfc3339(text).unwrap()
    }
    fn observed() -> UtcTimestamp {
        time("2026-10-13T07:30:00Z")
    }

    #[derive(Clone)]
    struct Batch {
        entry: ManifestEntry,
        bodies: BTreeMap<String, Vec<u8>>,
    }

    impl Batch {
        fn new(id: u128, day: &str) -> Self {
            Self {
                entry: ManifestEntry {
                    batch_id: BatchId::from_uuid(Uuid::from_u128(id)),
                    provider: "kis".to_owned(),
                    market: "kr".to_owned(),
                    date: date(day),
                    retrieved_at: time("2026-10-12T07:31:00Z"),
                    mode: FetchMode::Credentialed,
                    entitlement_reference: Some(ENTITLEMENT.to_owned()),
                    files: Vec::new(),
                },
                bodies: BTreeMap::new(),
            }
        }
        fn add(
            &mut self,
            kind: ResponseKind,
            name: &str,
            endpoint: &str,
            tr: &str,
            query: Vec<(String, String)>,
            body: Value,
        ) {
            let bytes = serde_json::to_vec(&body).unwrap();
            self.entry.files.push(FileEntry {
                kind,
                file_name: name.to_owned(),
                content_hash: ContentHash::from_bytes(&bytes),
                size_bytes: bytes.len() as u64,
                request: RequestMetadata {
                    endpoint: format!("/uapi/domestic-stock/v1/{endpoint}"),
                    query,
                    headers: vec![
                        ("appkey".to_owned(), "[REDACTED]".to_owned()),
                        ("appsecret".to_owned(), "[REDACTED]".to_owned()),
                        ("authorization".to_owned(), "[REDACTED]".to_owned()),
                        ("tr_cont".to_owned(), String::new()),
                        ("tr_id".to_owned(), tr.to_owned()),
                    ],
                    mode: FetchMode::Credentialed,
                },
                response_continuation: Some("M".to_owned()),
                copied_from: None,
            });
            self.bodies.insert(name.to_owned(), bytes);
        }
        fn body(&mut self, name: &str, bytes: Vec<u8>) {
            let file = self
                .entry
                .files
                .iter_mut()
                .find(|file| file.file_name == name)
                .unwrap();
            file.content_hash = ContentHash::from_bytes(&bytes);
            file.size_bytes = bytes.len() as u64;
            self.bodies.insert(name.to_owned(), bytes);
        }
        fn edit_body(&mut self, name: &str, edit: impl FnOnce(&mut Value)) {
            let mut value: Value = serde_json::from_slice(&self.bodies[name]).unwrap();
            edit(&mut value);
            self.body(name, serde_json::to_vec(&value).unwrap());
        }
        fn reference(&self, name: &str) -> SourceFileReference {
            let file = self
                .entry
                .files
                .iter()
                .find(|file| file.file_name == name)
                .unwrap();
            SourceFileReference {
                provider: self.entry.provider.clone(),
                market: self.entry.market.clone(),
                batch_id: self.entry.batch_id,
                file_name: name.to_owned(),
                content_hash: file.content_hash.clone(),
                retrieved_at: self.entry.retrieved_at,
            }
        }
    }

    fn price_batch(id: u128) -> Batch {
        let mut batch = Batch::new(id, DAY);
        for symbol in KR_ETF_CORE_SYMBOLS {
            batch.add(ResponseKind::Bars,&format!("bar-{symbol}.json"),"quotations/inquire-daily-itemchartprice","FHKST03010100",vec![("FID_COND_MRKT_DIV_CODE".into(),"J".into()),("FID_INPUT_ISCD".into(),symbol.into()),("FID_INPUT_DATE_1".into(),"20261012".into()),("FID_INPUT_DATE_2".into(),"20261012".into()),("FID_PERIOD_DIV_CODE".into(),"D".into()),("FID_ORG_ADJ_PRC".into(),"1".into())],json!({"rt_cd":"0","output1":{"prdt_name":"fixture-name-not-reconstructed"},"output2":[bar_row("20261012")]}));
            batch.add(
                ResponseKind::Reference,
                &format!("ref-{symbol}.json"),
                "quotations/inquire-price",
                "FHKST01010100",
                vec![
                    ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
                    ("FID_INPUT_ISCD".into(), symbol.into()),
                ],
                json!({"rt_cd":"0","output":{"stck_shrn_iscd":symbol}}),
            );
        }
        batch
    }

    fn bar_row(day: &str) -> Value {
        json!({"stck_bsop_date":day,"stck_oprc":"100.00","stck_hgpr":"120","stck_lwpr":"90.0","stck_clpr":"110","acml_vol":"200","acml_tr_pbmn":"22000"})
    }

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        files: Vec<PathBuf>,
        uid: u32,
    }

    fn private_dir(root: &Path, path: &Path) {
        let mut current = root.to_path_buf();
        for part in path.strip_prefix(root).unwrap().components() {
            current.push(part);
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
            let mut manifests: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
            for batch in batches {
                let dir = root.join(format!(
                    "raw/provider={}/market=kr/date={}/batch={}",
                    batch.entry.provider, batch.entry.date, batch.entry.batch_id
                ));
                write(
                    dir.join("batch.json"),
                    &serde_json::to_vec_pretty(&batch.entry).unwrap(),
                );
                for file in &batch.entry.files {
                    write(dir.join(&file.file_name), &batch.bodies[&file.file_name]);
                }
                let lines = manifests.entry(&batch.entry.provider).or_default();
                lines.extend(serde_json::to_vec(&batch.entry).unwrap());
                lines.push(b'\n');
            }
            for (provider, lines) in manifests {
                let manifest = root.join(format!("raw/manifests/provider={provider}/market=kr"));
                write(manifest.join("commit.lock"), b"");
                write(manifest.join("manifest.jsonl"), &lines);
            }
            let uid = fs::metadata(root.join("raw")).unwrap().uid();
            Self {
                _temp: temp,
                root,
                files,
                uid,
            }
        }
        fn scan(&self) -> CommittedRawSnapshot {
            self.scan_provider("kis")
        }
        fn scan_provider(&self, provider: &str) -> CommittedRawSnapshot {
            let values = ScanLimitValues {
                max_manifest_bytes: 2_097_152,
                max_manifest_records: 256,
                max_batches: 256,
                max_files_per_batch: 1024,
                max_files: 2048,
                max_batch_metadata_bytes: 1_048_576,
                max_source_file_bytes: 8_388_608,
                max_retained_bytes: 33_554_432,
                max_open_descriptors: 8192,
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
        fn fingerprint(&self) -> Vec<(ContentHash, u32, i64, i64, u64)> {
            self.files
                .iter()
                .map(|path| {
                    let m = fs::metadata(path).unwrap();
                    (
                        ContentHash::from_bytes(&fs::read(path).unwrap()),
                        m.mode(),
                        m.mtime(),
                        m.mtime_nsec(),
                        m.ino(),
                    )
                })
                .collect()
        }
    }

    struct ActionFixtureSpec {
        label: &'static str,
        path: &'static str,
        tr: &'static str,
        extras: &'static [(&'static str, &'static str)],
    }

    // Independent fixture matrix: class, provider filename, path, TR and extras.
    const ACTIONS: [ActionFixtureSpec; 7] = [
        ActionFixtureSpec {
            label: "corporate-actions-paidin-subscription",
            path: "paidin-capin",
            tr: "HHKDB669100C0",
            extras: &[("GB1", "1")],
        },
        ActionFixtureSpec {
            label: "corporate-actions-paidin-record",
            path: "paidin-capin",
            tr: "HHKDB669100C0",
            extras: &[("GB1", "2")],
        },
        ActionFixtureSpec {
            label: "corporate-actions-bonus",
            path: "bonus-issue",
            tr: "HHKDB669101C0",
            extras: &[],
        },
        ActionFixtureSpec {
            label: "corporate-actions-dividend",
            path: "dividend",
            tr: "HHKDB669102C0",
            extras: &[("GB1", "0"), ("HIGH_GB", "")],
        },
        ActionFixtureSpec {
            label: "corporate-actions-merger-split",
            path: "merger-split",
            tr: "HHKDB669104C0",
            extras: &[],
        },
        ActionFixtureSpec {
            label: "corporate-actions-reverse-split",
            path: "rev-split",
            tr: "HHKDB669105C0",
            extras: &[("MARKET_GB", "0")],
        },
        ActionFixtureSpec {
            label: "corporate-actions-capital-decrease",
            path: "cap-dcrs",
            tr: "HHKDB669106C0",
            extras: &[],
        },
    ];

    fn action_name(label: &str, symbol: Option<&str>, page: usize) -> String {
        match symbol {
            Some(symbol) => format!("{label}-{symbol}-page-{page:02}.json"),
            None => format!("{label}-page-{page:02}.json"),
        }
    }

    fn action_batch(id: u128, start: &str, end: &str, fixed: bool) -> Batch {
        let mut batch = Batch::new(id, start);
        let symbols: Vec<_> = if fixed {
            KR_ETF_CORE_SYMBOLS.into_iter().map(Some).collect()
        } else {
            vec![None]
        };
        for symbol in symbols {
            for ActionFixtureSpec {
                label,
                path,
                tr,
                extras,
            } in ACTIONS
            {
                let mut query = vec![
                    ("F_DT".into(), start.replace('-', "")),
                    ("T_DT".into(), end.replace('-', "")),
                    ("SHT_CD".into(), symbol.unwrap_or("").into()),
                    ("CTS".into(), String::new()),
                ];
                query.extend(
                    extras
                        .iter()
                        .map(|(key, value)| ((*key).into(), (*value).into())),
                );
                batch.add(
                    ResponseKind::CorporateActions,
                    &action_name(label, symbol, 1),
                    &format!("ksdinfo/{path}"),
                    tr,
                    query,
                    json!({"rt_cd":"0","output1":[],"fixture_page":1}),
                );
                batch.entry.files.last_mut().unwrap().response_continuation = Some("F".into());
            }
        }
        batch
    }

    fn request() -> SessionSourceRequest {
        SessionSourceRequest {
            owner: Uuid::from_u128(99),
            policy_sha256: ContentHash::from_bytes(b"fixture-policy"),
            previous_frontier: date("2026-10-09"),
            session_date: date(DAY),
        }
    }
    fn calendar_batch(id: u128, provider: &str, day: &str, rows: Value) -> Batch {
        let mut b = Batch::new(id, day);
        b.entry.provider = provider.into();
        b.add(
            ResponseKind::Calendar,
            "calendar.json",
            "quotations/chk-holiday",
            "CTCA0903R",
            vec![
                ("BASS_DT".into(), day.replace('-', "")),
                ("CTX_AREA_FK".into(), "".into()),
                ("CTX_AREA_NK".into(), "".into()),
            ],
            json!({"rt_cd":"0","output":rows}),
        );
        b
    }
    fn calendar_rows() -> Value {
        json!([{"bass_dt":"20261010","opnd_yn":"N"},{"bass_dt":"20261011","opnd_yn":"N"},{"bass_dt":"20261012","opnd_yn":"Y"}])
    }
    fn ready() -> Vec<Batch> {
        vec![
            price_batch(1),
            action_batch(2, "2026-10-10", DAY, false),
            calendar_batch(3, "kis", DAY, calendar_rows()),
        ]
    }
    fn discover(snapshot: &CommittedRawSnapshot) -> Result<DiscoveredSessionSources<'_>, E> {
        discover_session_sources(
            &RawEvidenceSnapshots::new(&[snapshot], ENTITLEMENT, observed()).unwrap(),
            &request(),
        )
    }
    fn fail(batches: &[Batch]) -> E {
        let f = Fixture::new(batches);
        let snapshot = f.scan();
        discover(&snapshot).err().unwrap()
    }

    fn committed_session(result: &DiscoveredSessionSources<'_>) -> DailySessionEvidenceV1Wire {
        let value: Value = serde_json::from_slice(result.evidence_preimage()).unwrap();
        serde_json::from_value(value["daily_sources"][0].clone()).unwrap()
    }

    #[test]
    fn daily_discovery_session_matches_canonical_commitment_order() {
        let f = Fixture::new(&ready());
        let snapshot = f.scan();
        let result = discover(&snapshot).unwrap();
        let committed = committed_session(&result);
        // Before the fix, the same seven verified chains compared unequal:
        // PaidinSubscription was first in discovery, BonusIssue in the wire.
        assert_eq!(result.session(), &committed);
        let labels: Vec<_> = result
            .session()
            .actions
            .iter()
            .map(|action| serde_json::to_value(action.logical_class).unwrap())
            .collect();
        assert_eq!(
            labels,
            vec![
                json!("bonus-issue"),
                json!("capital-decrease"),
                json!("dividend"),
                json!("merger-split"),
                json!("paidin-record"),
                json!("paidin-subscription"),
                json!("reverse-split"),
            ]
        );
        // Canonicalization does not alter reconstructed document commitments.
        let descriptors = &committed.normalized_documents;
        for document in result.documents() {
            assert!(descriptors.contains(document.descriptor()));
            assert_eq!(
                ContentHash::from_bytes(document.bytes()),
                document.descriptor().sha256
            );
        }
    }

    #[test]
    fn daily_discovery_later_observation_matches_committed_session_without_rewrite() {
        let f = Fixture::new(&ready());
        let before = f.fingerprint();
        let snapshot = f.scan();
        let original = discover(&snapshot).unwrap();
        let committed = committed_session(&original);
        let later_at = time("2026-10-13T08:31:00Z");
        let later = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, later_at).unwrap();
        let refreshed = discover_session_sources(&later, &request()).unwrap();
        assert_eq!(refreshed.session(), &committed);
        assert_eq!(refreshed.evidence_preimage(), original.evidence_preimage());
        let observed_files = refreshed.observation().files();
        assert_eq!(observed_files, original.observation().files());
        assert_eq!(f.fingerprint(), before);
    }

    #[test]
    fn daily_discovery_canonical_order_keeps_changed_source_distinct() {
        let f = Fixture::new(&ready());
        let snapshot = f.scan();
        let original = discover(&snapshot).unwrap();
        let committed = committed_session(&original);
        let mut changed = ready();
        changed[0].edit_body("bar-069500.json", |body| {
            body["output2"][0]["stck_clpr"] = json!("111");
            body["output2"][0]["acml_tr_pbmn"] = json!("22200");
        });
        let changed = Fixture::new(&changed);
        let snapshot = changed.scan();
        let refreshed = discover(&snapshot).unwrap();
        assert_eq!(refreshed.session(), &committed_session(&refreshed));
        assert_ne!(refreshed.session(), &committed);
        assert_ne!(refreshed.evidence_preimage(), original.evidence_preimage());
        let observed_hash = refreshed.observation().sha256();
        assert_ne!(observed_hash, original.observation().sha256());
    }

    #[test]
    fn daily_wholeflow_fixture_both_sessions_roundtrip_through_rust_discovery() {
        // Execute only the pure synthetic planner in hosted CI. These exact Raw
        // bytes exercise the Rust reader/discovery boundary, not V3 approval,
        // filesystem admission, SQL or the actual host whole-flow path.
        let planner = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py");
        let output = std::process::Command::new("python3")
            .args([
                "-B",
                "-c",
                "import datetime,json,runpy,sys; f=runpy.run_path(sys.argv[1]); p=f['plan'](datetime.datetime(2026,10,19,1,24,tzinfo=datetime.timezone.utc),'00000000-0000-4000-8000-000000000001','a'*40); print(json.dumps({k:v.decode('ascii') for k,v in p['raw_files'].items()}))",
            ])
            .arg(planner)
            .output()
            .unwrap();
        assert!(output.status.success(), "synthetic planner failed");
        assert!(output.stderr.is_empty());
        let raw: BTreeMap<String, String> = serde_json::from_slice(&output.stdout).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        let mut files = Vec::new();
        for (name, bytes) in raw {
            let path = root.join("raw").join(name);
            private_dir(&root, path.parent().unwrap());
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            files.push(path);
        }
        let uid = fs::metadata(root.join("raw")).unwrap().uid();
        let f = Fixture {
            _temp: temp,
            root,
            files,
            uid,
        };
        let before = f.fingerprint();
        let kis = f.scan();
        let calendar = f.scan_provider("kis-calendar");
        let snapshots = RawEvidenceSnapshots::new(
            &[&kis, &calendar],
            "fixture-only:synthetic-wholeflow-extension-not-market-receipt",
            time("2026-10-19T01:24:01Z"),
        )
        .unwrap();
        let owner = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let policy = ContentHash::from_bytes(include_bytes!(
            "../../../configs/evidence/owner-beta-daily-price-policy-v1.json"
        ));
        let mut previous = "2026-08-28";
        let mut committed = Vec::new();
        for target in ["2026-08-31", "2026-10-18"] {
            let request = SessionSourceRequest {
                owner,
                policy_sha256: policy.clone(),
                previous_frontier: date(previous),
                session_date: date(target),
            };
            let found = discover_session_sources(&snapshots, &request).unwrap();
            assert_eq!(found.session(), &committed_session(&found));
            assert_eq!(found.session().actions.len(), 7);
            assert_eq!(found.session().bars.len(), 11);
            committed.push(committed_session(&found));
            previous = target;
        }
        assert_eq!(committed[1].previous_frontier, committed[0].session_date);
        assert_ne!(committed[0], committed[1]);
        // A second poll must reconstruct exactly the stored first session.
        let refreshed = discover_session_sources(
            &snapshots,
            &SessionSourceRequest {
                owner,
                policy_sha256: policy,
                previous_frontier: date("2026-08-28"),
                session_date: date("2026-08-31"),
            },
        )
        .unwrap();
        assert_eq!(refreshed.session(), &committed[0]);
        assert_eq!(f.fingerprint(), before);
    }

    #[test]
    fn s5_complete_discovery_builds_four_kinds_and_exact_d4_session_commitment() {
        let f = Fixture::new(&ready());
        let snapshot = f.scan();
        let result = discover(&snapshot).unwrap();
        assert_eq!(result.owner(), request().owner);
        assert_eq!(result.policy_sha256(), &request().policy_sha256);
        assert_eq!(result.session().bars.len(), 11);
        assert_eq!(result.session().reference.len(), 11);
        assert_eq!(result.session().actions.len(), 7);
        assert_eq!(result.session().calendar.len(), 3);
        assert_eq!(result.documents().len(), 6);
        assert_eq!(result.observation().files().len(), 30);
        assert_eq!(
            result.evidence_preimage(),
            daily_price_evidence_preimage_v1(std::slice::from_ref(result.session())).unwrap()
        );
        for document in result.documents() {
            assert_eq!(
                ContentHash::from_bytes(document.bytes()),
                document.descriptor().sha256
            );
            assert_eq!(
                document.bytes().len() as u64,
                document.descriptor().size_bytes
            );
            let value: Value = serde_json::from_slice(document.bytes()).unwrap();
            assert_eq!(
                value["rows"].as_array().unwrap().len() as u64,
                document.descriptor().row_count
            );
            assert_eq!(
                value["source_evidence_sha256"],
                document.descriptor().source_evidence_sha256.as_str()
            );
        }
        assert_eq!(
            result
                .session()
                .calendar
                .iter()
                .map(|b| b.session_type)
                .collect::<Vec<_>>(),
            vec![
                DailySessionTypeV1::Closed,
                DailySessionTypeV1::Closed,
                DailySessionTypeV1::Trading
            ]
        );
        assert!(
            result
                .session()
                .calendar
                .iter()
                .all(|b| b.legacy_projection.is_none())
        );
        assert_eq!(result.prices().bars().descriptor().row_count, 11);
        assert_eq!(result.actions().witnesses().len(), 7);
    }
    #[test]
    fn s5_producer_cannot_hide_equal_or_conflicting_price_alternatives() {
        for conflict in [false, true] {
            let mut batches = ready();
            let mut extra = price_batch(4);
            if conflict {
                extra.edit_body("bar-069500.json", |body| {
                    body["output2"][0]["stck_clpr"] = json!("111")
                });
            }
            batches.push(extra);
            assert_eq!(fail(&batches), E::SourceConflict);
        }
    }
    #[test]
    fn s5_reference_alternative_is_detected_independently_of_bars() {
        let mut batches = ready();
        let mut extra = price_batch(4);
        extra
            .entry
            .files
            .retain(|f| f.kind == ResponseKind::Reference);
        batches.push(extra);
        assert_eq!(fail(&batches), E::SourceConflict);
    }
    #[test]
    fn s5_missing_price_or_reference_never_becomes_partial_success() {
        for kind in [ResponseKind::Bars, ResponseKind::Reference] {
            let mut batches = ready();
            let index = batches[0]
                .entry
                .files
                .iter()
                .position(|f| f.kind == kind)
                .unwrap();
            batches[0].entry.files.remove(index);
            assert_eq!(fail(&batches), E::SourceCoverageGap);
        }
    }
    #[test]
    fn s5_calendar_gap_and_intervening_trading_day_are_distinct_failures() {
        let mut gap = ready();
        gap[2].edit_body("calendar.json", |body| {
            body["output"].as_array_mut().unwrap().remove(1);
        });
        assert_eq!(fail(&gap), E::CalendarCoverageGap);
        let mut trading = ready();
        trading[2].edit_body("calendar.json", |body| {
            body["output"][1]["opnd_yn"] = json!("Y")
        });
        assert_eq!(fail(&trading), E::CalendarConflict);
        let mut closed = ready();
        closed[2].edit_body("calendar.json", |body| {
            body["output"][2]["opnd_yn"] = json!("N")
        });
        assert_eq!(fail(&closed), E::CalendarConflict);
    }
    #[test]
    fn s5_equal_calendar_witnesses_survive_deterministic_selected_binding() {
        let mut batches = ready();
        batches.push(calendar_batch(4, "kis", DAY, calendar_rows()));
        let f = Fixture::new(&batches);
        let snapshot = f.scan();
        let result = discover(&snapshot).unwrap();
        assert_eq!(result.calendars().len(), 2);
        assert_eq!(result.observation().files().len(), 31);
        let smallest = result
            .calendars()
            .iter()
            .min_by_key(|p| p.evidence_sha256().as_str())
            .unwrap();
        assert!(
            result
                .session()
                .calendar
                .iter()
                .all(|b| b.source_file == *smallest.source_file())
        );
        let old = Fixture::new(&ready());
        let old_snapshot = old.scan();
        let old_result = discover(&old_snapshot).unwrap();
        let event = compare_source_observation(
            request().owner,
            date("2026-10-10"),
            date(DAY),
            old_result.observation(),
            result.observation().files(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.code(), DailyBlockCode::SealedSourceChanged);
    }
    #[test]
    fn legacy_unlinked_calendar_duplicate_never_becomes_a_witness() {
        use market_data::daily_price_only::calendar::project_calendar;
        use market_data::daily_price_only::verify::resolve_original;
        let base = Fixture::new(&ready());
        let base_snapshot = base.scan();
        let expected = discover(&base_snapshot).unwrap();
        let mut batches = ready();
        let mut cached = batches[2].clone();
        cached.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(4));
        cached.entry.date = date("2026-10-13");
        cached.entry.retrieved_at = observed();
        let cached_ref = cached.reference("calendar.json");
        batches.push(cached);
        let f = Fixture::new(&batches);
        let before = f.fingerprint();
        let snapshot = f.scan();
        let context = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        let result = discover_session_sources(&context, &request()).unwrap();
        assert_eq!(result.evidence_preimage(), expected.evidence_preimage());
        assert_eq!(result.calendars().len(), 1);
        assert_eq!(result.observation().files(), expected.observation().files());
        let cached = resolve_original(&context, &cached_ref).unwrap();
        // Recognition of a redundant delivery is not a synthetic copy chain.
        assert_eq!(
            project_calendar(&cached, request().owner, &request().policy_sha256).err(),
            Some(E::InvalidEvidence)
        );
        assert_eq!(f.fingerprint(), before);
    }

    #[test]
    fn legacy_unlinked_calendar_unknown_or_conflicting_sources_still_block() {
        for variant in 0..6 {
            let mut batches = ready();
            let mut cached = batches[2].clone();
            cached.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(4));
            cached.entry.date = date("2026-10-13");
            cached.entry.retrieved_at = observed();
            match variant {
                0 => {
                    batches.pop();
                } // No verified original.
                1 => cached.edit_body("calendar.json", |body| {
                    body["output"][0]["opnd_yn"] = json!("Y")
                }),
                2 => cached.entry.files[0].request.query[1].1 = "changed".into(),
                3 => cached.entry.retrieved_at = time("2026-10-12T06:30:00Z"),
                4 => cached.entry.date = date("2026-10-11"),
                5 => {
                    cached.entry.files[0].copied_from = Some(SourceFileReference {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(999)),
                        ..batches[2].reference("calendar.json")
                    })
                }
                _ => unreachable!(),
            }
            batches.push(cached);
            let f = Fixture::new(&batches);
            let before = f.fingerprint();
            let snapshot = f.scan();
            assert!(discover(&snapshot).is_err(), "variant {variant}");
            assert_eq!(f.fingerprint(), before);
        }
    }
    #[test]
    fn discovery_entitlement_selection_never_authorizes_foreign_evidence() {
        use market_data::daily_price_only::verify::resolve_original;
        let expected_fixture = Fixture::new(&ready());
        let expected_snapshot = expected_fixture.scan();
        let expected = discover(&expected_snapshot).unwrap();
        let mut batches = ready();
        let mut foreign = ready();
        for (index, batch) in foreign.iter_mut().enumerate() {
            batch.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(10 + index as u128));
            batch.entry.entitlement_reference = Some("fixture:other-owner".into());
        }
        foreign[2].edit_body("calendar.json", |body| {
            body["output"][0]["opnd_yn"] = json!("Y")
        });
        let foreign_ref = foreign[2].reference("calendar.json");
        batches.extend(foreign.clone());
        let f = Fixture::new(&batches);
        let before = f.fingerprint();
        let snapshot = f.scan();
        let context = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        let result = discover_session_sources(&context, &request()).unwrap();
        assert_eq!(result.evidence_preimage(), expected.evidence_preimage());
        assert_eq!(result.observation().files(), expected.observation().files());
        assert_eq!(
            resolve_original(&context, &foreign_ref).err(),
            Some(E::EntitlementMismatch)
        );
        assert_eq!(f.fingerprint(), before);

        // Removing the entitled evidence cannot fall back to the other owner.
        assert_eq!(fail(&foreign), E::SourceCoverageGap);
    }

    #[test]
    fn entitled_delivery_cannot_copy_from_another_entitlement() {
        let mut batches = ready();
        let mut foreign = batches[2].clone();
        foreign.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(10));
        foreign.entry.entitlement_reference = Some("fixture:other-owner".into());
        batches[2].entry.files[0].copied_from = Some(foreign.reference("calendar.json"));
        batches.push(foreign);
        assert_eq!(fail(&batches), E::EntitlementMismatch);
    }
    #[test]
    fn s5_cross_scope_calendar_conflict_cannot_be_hidden() {
        let mut batches = ready();
        let mut rows = calendar_rows();
        rows[1]["opnd_yn"] = json!("Y");
        batches.push(calendar_batch(4, "kis-calendar", DAY, rows));
        let f = Fixture::new(&batches);
        let kis = f.scan();
        let calendar = f.scan_provider("kis-calendar");
        let context =
            RawEvidenceSnapshots::new(&[&kis, &calendar], ENTITLEMENT, observed()).unwrap();
        assert_eq!(
            discover_session_sources(&context, &request()).err(),
            Some(E::CalendarConflict)
        );
    }
    #[test]
    fn s5_cross_scope_copy_keeps_original_target_and_all_witnesses() {
        let mut batches = ready();
        let original = &batches[2];
        let mut copy = original.clone();
        copy.entry.provider = "kis-calendar".into();
        copy.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(4));
        copy.entry.date = date("2026-10-13");
        copy.entry.retrieved_at = observed();
        copy.entry.files[0].copied_from = Some(original.reference("calendar.json"));
        batches.push(copy);
        let f = Fixture::new(&batches);
        let kis = f.scan();
        let calendar = f.scan_provider("kis-calendar");
        let context =
            RawEvidenceSnapshots::new(&[&kis, &calendar], ENTITLEMENT, observed()).unwrap();
        let result = discover_session_sources(&context, &request()).unwrap();
        assert_eq!(result.calendars().len(), 2);
        let copied = result
            .calendars()
            .iter()
            .find(|p| p.source_file().provider == "kis-calendar")
            .unwrap();
        assert_eq!(copied.source_file().copied_from[0].batch_date, date(DAY));
        assert_eq!(result.observation().files().len(), 31);
    }
    #[test]
    fn s5_unrelated_valid_calendar_and_action_append_leave_frontier_unchanged() {
        let base = Fixture::new(&ready());
        let base_snapshot = base.scan();
        let expected = discover(&base_snapshot).unwrap();
        let mut batches = ready();
        batches.push(calendar_batch(
            4,
            "kis",
            "2026-09-01",
            json!({"bass_dt":"20260901","opnd_yn":"Y"}),
        ));
        batches.push(action_batch(5, "2026-09-01", "2026-09-01", false));
        let f = Fixture::new(&batches);
        let snapshot = f.scan();
        let actual = discover(&snapshot).unwrap();
        assert_eq!(
            expected.observation().preimage(),
            actual.observation().preimage()
        );
        assert_eq!(expected.evidence_preimage(), actual.evidence_preimage());
    }
    #[test]
    fn s5_surrounding_and_other_daily_bar_rows_are_retained_for_prefix_comparison() {
        let mut batches = ready();
        batches[0].edit_body("bar-069500.json", |body| {
            body["output2"]
                .as_array_mut()
                .unwrap()
                .push(bar_row("20261009"))
        });
        let mut prior = price_batch(4);
        prior.entry.date = date("2026-10-09");
        prior.entry.files.retain(|f| f.kind == ResponseKind::Bars);
        let names: Vec<_> = prior
            .entry
            .files
            .iter()
            .map(|f| f.file_name.clone())
            .collect();
        for file in &mut prior.entry.files {
            for (key, value) in &mut file.request.query {
                if key == "FID_INPUT_DATE_1" || key == "FID_INPUT_DATE_2" {
                    *value = "20261009".into();
                }
            }
        }
        for name in names {
            prior.edit_body(&name, |body| body["output2"] = json!([bar_row("20261009")]));
        }
        batches.push(prior);
        let f = Fixture::new(&batches);
        let snapshot = f.scan();
        let result = discover(&snapshot).unwrap();
        assert_eq!(result.observed_bars().len(), 22);
        assert_eq!(result.additional_original_count(), 11);
        assert_eq!(
            result
                .observed_bars()
                .iter()
                .map(|f| f.rows().len())
                .sum::<usize>(),
            23
        );
        let target = result
            .observed_bars()
            .iter()
            .find(|r| {
                r.source().batch_id == BatchId::from_uuid(Uuid::from_u128(1))
                    && r.source().file_name == "bar-069500.json"
            })
            .unwrap();
        assert_eq!(target.rows()[0]["date"], "2026-10-09");
        assert_eq!(target.rows()[1]["date"], DAY);
        assert_eq!(result.prices().bars().descriptor().row_count, 11);
    }
    #[test]
    fn s5_malformed_surrounding_bar_does_not_disappear_during_discovery() {
        let mut batches = ready();
        batches[0].edit_body("bar-069500.json", |body| {
            let mut bad = bar_row("20261009");
            bad["stck_oprc"] = json!("malformed");
            body["output2"].as_array_mut().unwrap().push(bad);
        });
        assert_eq!(fail(&batches), E::InvalidBars);
    }
    #[test]
    fn s5_copied_price_is_not_a_new_independent_target_identity() {
        let mut batches = ready();
        let original = &batches[0];
        let mut copy = original.clone();
        copy.entry.batch_id = BatchId::from_uuid(Uuid::from_u128(4));
        copy.entry.date = date("2026-10-13");
        copy.entry.retrieved_at = observed();
        for file in &mut copy.entry.files {
            file.copied_from = Some(original.reference(&file.file_name));
        }
        batches.push(copy);
        assert_eq!(fail(&batches), E::SourceConflict);
    }
    #[test]
    fn s5_manifest_order_does_not_change_source_or_document_identity() {
        let first = Fixture::new(&ready());
        let first_snapshot = first.scan();
        let expected = discover(&first_snapshot).unwrap();
        let mut batches = ready();
        batches.reverse();
        let second = Fixture::new(&batches);
        let second_snapshot = second.scan();
        let actual = discover(&second_snapshot).unwrap();
        assert_eq!(
            expected.observation().preimage(),
            actual.observation().preimage()
        );
        assert_eq!(expected.evidence_preimage(), actual.evidence_preimage());
        assert_eq!(
            expected
                .documents()
                .iter()
                .map(|d| d.bytes())
                .collect::<Vec<_>>(),
            actual
                .documents()
                .iter()
                .map(|d| d.bytes())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn s5_future_cutoff_invalid_identity_and_oversized_extension_fail_before_discovery() {
        let f = Fixture::new(&ready());
        let snapshot = f.scan();
        let context =
            RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, time("2026-10-12T07:29:59Z"))
                .unwrap();
        assert_eq!(
            discover_session_sources(&context, &request()).err(),
            Some(E::InvalidContext)
        );
        let context = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        let mut nil = request();
        nil.owner = Uuid::nil();
        assert_eq!(
            discover_session_sources(&context, &nil).err(),
            Some(E::InvalidContext)
        );
        let mut reverse = request();
        reverse.previous_frontier = reverse.session_date;
        assert_eq!(
            discover_session_sources(&context, &reverse).err(),
            Some(E::InvalidContext)
        );
        let mut wide = request();
        wide.session_date = date("2028-10-12");
        assert_eq!(
            discover_session_sources(&context, &wide).err(),
            Some(E::InvalidContext)
        );
    }
    #[test]
    fn s5_complete_action_discovery_preserves_gap_and_noncash_failures() {
        let mut gap = ready();
        gap[1].entry.files.pop();
        assert_eq!(fail(&gap), E::SourceCoverageGap);
        let mut noncash = ready();
        noncash[1].edit_body("corporate-actions-bonus-page-01.json",|body|body["output1"]=json!([{"sht_cd":"069500","record_date":"20261012","right_dt":"20261013","fix_rate":"1"}]));
        assert_eq!(fail(&noncash), E::NonCashAction);
    }
    #[test]
    fn s5_discovery_is_read_only_and_retains_shared_snapshot_lock() {
        let f = Fixture::new(&ready());
        let before = f.fingerprint();
        let snapshot = f.scan();
        let result = discover(&snapshot).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(
                f.root
                    .join("raw/manifests/provider=kis/market=kr/commit.lock"),
            )
            .unwrap();
        assert_eq!(
            lock.try_lock_exclusive().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(before, f.fingerprint());
        drop(result);
        drop(snapshot);
        lock.try_lock_exclusive().unwrap();
        FileExt::unlock(&lock).unwrap();
    }

    #[test]
    fn s5_unrelated_non_etf_price_delivery_is_outside_fixed_universe() {
        let base = Fixture::new(&ready());
        let base_snapshot = base.scan();
        let expected = discover(&base_snapshot).unwrap();
        let mut batches = ready();
        let mut stock = price_batch(4);
        stock
            .entry
            .files
            .retain(|f| f.file_name == "bar-069500.json" || f.file_name == "ref-069500.json");
        for file in &mut stock.entry.files {
            for (key, value) in &mut file.request.query {
                if key == "FID_INPUT_ISCD" {
                    *value = "000020".into();
                }
            }
        }
        stock.edit_body("ref-069500.json", |body| {
            body["output"]["stck_shrn_iscd"] = json!("000020")
        });
        batches.push(stock);
        let f = Fixture::new(&batches);
        let snapshot = f.scan();
        let actual = discover(&snapshot).unwrap();
        assert_eq!(
            actual.observation().preimage(),
            expected.observation().preimage()
        );
        assert_eq!(actual.observed_bars().len(), 11);
    }
}
