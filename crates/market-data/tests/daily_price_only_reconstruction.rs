//! Synthetic committed provider evidence; no live requests or DB authority.

#[cfg(target_os = "linux")]
mod linux_tests {
    use std::collections::BTreeMap;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
    use fs2::FileExt;
    use market_data::contract::{FetchMode, RequestMetadata, ResponseKind, SourceFileReference};
    use market_data::daily_price_only::reconstruct::{
        VerifiedActionDocument, VerifiedPriceDocuments, reconstruct_actions, reconstruct_prices,
    };
    use market_data::daily_price_only::source::{
        ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, ScanLimitValues, ScanLimits,
    };
    use market_data::daily_price_only::verify::{
        DailyVerificationError as E, RawEvidenceSnapshots, ResolvedOriginal, resolve_original,
    };
    use market_data::daily_price_only::{
        DailyActionClassV1 as Class, DailyActionScopeV1 as Scope,
        DailyNormalizedSourceCommitmentV1Wire, daily_cash_dividend_rows_preimage_v1,
        daily_normalized_source_preimage_v1,
    };
    use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
    use market_data::storage::{FileEntry, ManifestEntry};
    use serde_json::{Value, json};
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
            let manifest = root.join("raw/manifests/provider=kis/market=kr");
            write(manifest.join("commit.lock"), b"");
            let mut lines = Vec::new();
            for batch in batches {
                let dir = root.join(format!(
                    "raw/provider=kis/market=kr/date={}/batch={}",
                    batch.entry.date, batch.entry.batch_id
                ));
                write(
                    dir.join("batch.json"),
                    &serde_json::to_vec_pretty(&batch.entry).unwrap(),
                );
                for file in &batch.entry.files {
                    write(dir.join(&file.file_name), &batch.bodies[&file.file_name]);
                }
                lines.extend(serde_json::to_vec(&batch.entry).unwrap());
                lines.push(b'\n');
            }
            write(manifest.join("manifest.jsonl"), &lines);
            let uid = fs::metadata(root.join("raw")).unwrap().uid();
            Self {
                _temp: temp,
                root,
                files,
                uid,
            }
        }
        fn scan(&self) -> CommittedRawSnapshot {
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
                    ApprovedRawScope::new("kis", "kr", self.uid).unwrap(),
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

    fn resolve_kind(
        snapshot: &CommittedRawSnapshot,
        kind: ResponseKind,
    ) -> Vec<ResolvedOriginal<'_>> {
        let scopes = RawEvidenceSnapshots::new(&[snapshot], ENTITLEMENT, observed()).unwrap();
        snapshot
            .batches()
            .iter()
            .flat_map(|batch| batch.files())
            .filter(|file| file.kind() == kind)
            .map(|file| {
                let reference = SourceFileReference {
                    provider: "kis".into(),
                    market: "kr".into(),
                    batch_id: file.batch().batch_id(),
                    file_name: file.file_name().into(),
                    content_hash: file.content_hash().clone(),
                    retrieved_at: file.batch().retrieved_at(),
                };
                resolve_original(&scopes, &reference).unwrap()
            })
            .collect()
    }

    fn prices(snapshot: &CommittedRawSnapshot) -> Result<VerifiedPriceDocuments<'_>, E> {
        reconstruct_prices(
            date(DAY),
            &resolve_kind(snapshot, ResponseKind::Bars),
            &resolve_kind(snapshot, ResponseKind::Reference),
        )
    }

    fn failure(batch: Batch) -> E {
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        prices(&snapshot).err().unwrap()
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

    fn cash_row(symbol: &str, day: &str) -> Value {
        json!({"sht_cd":symbol,"record_date":day.replace('-',""),"divi_kind":"","face_val":"1000.00","per_sto_divi_amt":"12.3400","divi_rate":"0.123400","stk_divi_rate":"0.00","divi_pay_dt":"20261019","stk_div_pay_dt":"","odd_pay_dt":"","stk_kind":"","high_divi_gb":""})
    }

    fn put_rows(batch: &mut Batch, label: &str, symbol: Option<&str>, rows: Value) {
        batch.edit_body(&action_name(label, symbol, 1), |body| {
            body["output1"] = rows
        });
    }

    fn append_page(
        batch: &mut Batch,
        label: &str,
        symbol: Option<&str>,
        ordinal: usize,
        rows: Value,
    ) {
        let first = action_name(label, symbol, 1);
        let previous = action_name(label, symbol, ordinal - 1);
        batch
            .entry
            .files
            .iter_mut()
            .find(|file| file.file_name == previous)
            .unwrap()
            .response_continuation = Some("M".into());
        let mut file = batch
            .entry
            .files
            .iter()
            .find(|file| file.file_name == first)
            .unwrap()
            .clone();
        file.file_name = action_name(label, symbol, ordinal);
        file.response_continuation = Some("F".into());
        file.request
            .headers
            .iter_mut()
            .find(|(name, _)| name == "tr_cont")
            .unwrap()
            .1 = "N".into();
        let bytes = serde_json::to_vec(&json!({"rt_cd":"0","output1":rows,"fixture_page":ordinal}))
            .unwrap();
        file.size_bytes = bytes.len() as u64;
        file.content_hash = ContentHash::from_bytes(&bytes);
        batch.bodies.insert(file.file_name.clone(), bytes);
        batch.entry.files.push(file);
    }

    fn actions<'a>(
        snapshot: &'a CommittedRawSnapshot,
        previous: &str,
    ) -> Result<VerifiedActionDocument<'a>, E> {
        let context = RawEvidenceSnapshots::new(&[snapshot], ENTITLEMENT, observed()).unwrap();
        reconstruct_actions(&context, date(previous), date(DAY))
    }

    fn action_failure(batches: &[Batch], previous: &str) -> E {
        let fixture = Fixture::new(batches);
        let snapshot = fixture.scan();
        actions(&snapshot, previous).err().unwrap()
    }

    #[test]
    fn s4_prices_cover_all_eleven_symbols_and_exact_canonical_fields() {
        let fixture = Fixture::new(&[price_batch(1)]);
        let snapshot = fixture.scan();
        let result = prices(&snapshot).unwrap();
        let mut symbols = KR_ETF_CORE_SYMBOLS;
        symbols.sort();
        assert_eq!(result.bars().rows().len(), 11);
        assert_eq!(result.reference().rows().len(), 11);
        for (index, symbol) in symbols.iter().enumerate() {
            assert_eq!(
                result.bars().rows()[index],
                json!({"instrument":format!("{symbol}.KRX"),"date":DAY,"open":100.0,"high":120,"low":90.0,"close":110,"volume":200,"value":22000})
            );
            assert_eq!(
                result.reference().rows()[index],
                json!({"instrument":format!("{symbol}.KRX"),"provider_symbol":symbol})
            );
        }
        for document in [result.bars(), result.reference()] {
            assert_eq!(document.descriptor().row_count, 11);
            assert_eq!(document.descriptor().source_files.len(), 11);
            assert_eq!(
                document.descriptor().sha256,
                ContentHash::from_bytes(document.bytes())
            );
            assert_eq!(
                document.descriptor().size_bytes,
                document.bytes().len() as u64
            );
            assert!(!document.bytes().ends_with(b"\n"));
            assert!(
                !std::str::from_utf8(document.bytes())
                    .unwrap()
                    .contains("fixture-name")
            );
        }
    }

    #[test]
    fn s4_missing_or_duplicate_request_symbol_cannot_cover_the_universe() {
        let mut missing = price_batch(1);
        missing.entry.files.pop();
        assert_eq!(failure(missing), E::SourceCoverageGap);
        let mut duplicate = price_batch(1);
        let file = duplicate
            .entry
            .files
            .iter_mut()
            .find(|file| file.file_name == "bar-102110.json")
            .unwrap();
        file.request
            .query
            .iter_mut()
            .find(|(key, _)| key == "FID_INPUT_ISCD")
            .unwrap()
            .1 = "069500".into();
        assert_eq!(failure(duplicate), E::SourceConflict);
    }

    #[test]
    fn s4_bar_request_must_be_exact_unadjusted_single_session() {
        for (key, value) in [
            ("FID_ORG_ADJ_PRC", "0"),
            ("FID_INPUT_DATE_1", "20261009"),
            ("FID_INPUT_DATE_2", "20261013"),
            ("FID_PERIOD_DIV_CODE", "W"),
        ] {
            let mut batch = price_batch(1);
            batch.entry.files[0]
                .request
                .query
                .iter_mut()
                .find(|(name, _)| name == key)
                .unwrap()
                .1 = value.into();
            assert_eq!(failure(batch), E::InvalidEvidence);
        }
        let mut batch = price_batch(1);
        batch.entry.files[0]
            .request
            .headers
            .iter_mut()
            .find(|(name, _)| name == "tr_cont")
            .unwrap()
            .1 = "N".into();
        assert_eq!(failure(batch), E::InvalidEvidence);
    }

    #[test]
    fn s4_reference_requires_its_own_returned_identity() {
        for value in [Value::Null, json!("102110"), json!(69500)] {
            let mut batch = price_batch(1);
            batch.edit_body("ref-069500.json", |body| {
                body["output"]["stck_shrn_iscd"] = value
            });
            assert_eq!(failure(batch), E::InvalidReference);
        }
        let mut batch = price_batch(1);
        batch.edit_body("ref-069500.json", |body| {
            body["output"]
                .as_object_mut()
                .unwrap()
                .remove("stck_shrn_iscd");
            body["output"]["prdt_name"] = json!("a name is not identity");
        });
        assert_eq!(failure(batch), E::InvalidReference);
    }

    #[test]
    fn s4_every_surrounding_bar_is_validated_before_target_selection() {
        for bad in [
            json!({"stck_bsop_date":"20261009"}),
            json!({"stck_bsop_date":"20260230"}),
            Value::Null,
        ] {
            let mut batch = price_batch(1);
            batch.edit_body("bar-069500.json", |body| {
                body["output2"].as_array_mut().unwrap().push(bad)
            });
            assert_eq!(failure(batch), E::InvalidBars);
        }
        let mut batch = price_batch(1);
        batch.edit_body("bar-069500.json", |body| {
            let rows = body["output2"].as_array_mut().unwrap();
            rows.push(bar_row("20261009"));
            rows.push(bar_row("20261013"));
        });
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        assert_eq!(prices(&snapshot).unwrap().bars().rows().len(), 11);
    }

    #[test]
    fn s4_absent_and_duplicate_target_rows_fail_closed() {
        let mut missing = price_batch(1);
        missing.edit_body("bar-069500.json", |body| {
            body["output2"] = json!([bar_row("20261009")])
        });
        assert_eq!(failure(missing), E::SourceCoverageGap);
        let mut duplicate = price_batch(1);
        duplicate.edit_body("bar-069500.json", |body| {
            body["output2"]
                .as_array_mut()
                .unwrap()
                .push(bar_row("20261012"))
        });
        assert_eq!(failure(duplicate), E::SourceConflict);
        let mut surrounding = price_batch(1);
        surrounding.edit_body("bar-069500.json", |body| {
            let rows = body["output2"].as_array_mut().unwrap();
            rows.push(bar_row("20261009"));
            rows.push(bar_row("20261009"));
        });
        assert_eq!(failure(surrounding), E::SourceConflict);
    }

    #[test]
    fn s4_optional_value_is_preserved_or_absent_without_invention() {
        let mut batch = price_batch(1);
        batch.edit_body("bar-069500.json", |body| {
            body["output2"][0]
                .as_object_mut()
                .unwrap()
                .remove("acml_tr_pbmn");
        });
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        assert!(
            prices(&snapshot).unwrap().bars().rows()[0]
                .get("value")
                .is_none()
        );
        let mut batch = price_batch(1);
        batch.edit_body("bar-069500.json", |body| {
            body["output2"][0]["acml_tr_pbmn"] = json!("not-a-number")
        });
        assert_eq!(failure(batch), E::InvalidBars);
    }

    #[test]
    fn s4_provider_shapes_json_duplicates_and_error_messages_are_rejected() {
        for bytes in [
            br#"{"rt_cd":"0","rt_cd":"0","output":{}}"#.as_slice(),
            br#"{"rt_cd":"0","output":{},"output":oops}"#.as_slice(),
            br#"{"rt_cd":"0","output":{}} {}"#.as_slice(),
        ] {
            let mut batch = price_batch(1);
            batch.body("ref-069500.json", bytes.to_vec());
            assert_eq!(failure(batch), E::InvalidProviderJson);
        }
        let mut batch = price_batch(1);
        batch.edit_body("ref-069500.json", |body| {
            body["rt_cd"] = json!("1");
            body["msg1"] = json!("FIXTURE_SENSITIVE_MESSAGE");
        });
        let error = failure(batch);
        assert_eq!(error, E::ProviderResponse);
        assert_eq!(error.to_string(), "PROVIDER_RESPONSE");
        let mut batch = price_batch(1);
        batch.edit_body("bar-069500.json", |body| body["output1"] = Value::Null);
        assert_eq!(failure(batch), E::InvalidBars);
    }

    #[test]
    fn s4_input_order_does_not_change_documents_or_source_order() {
        let fixture = Fixture::new(&[price_batch(1)]);
        let snapshot = fixture.scan();
        let mut bars = resolve_kind(&snapshot, ResponseKind::Bars);
        let mut references = resolve_kind(&snapshot, ResponseKind::Reference);
        let first = reconstruct_prices(date(DAY), &bars, &references).unwrap();
        bars.reverse();
        references.rotate_left(4);
        let again = reconstruct_prices(date(DAY), &bars, &references).unwrap();
        assert_eq!(first.bars().bytes(), again.bars().bytes());
        assert_eq!(first.reference().bytes(), again.reference().bytes());
        assert_eq!(first.bar_sources(), again.bar_sources());
        assert_eq!(first.reference_sources(), again.reference_sources());
    }

    #[test]
    fn s4_copy_uses_original_session_and_retains_every_commitment() {
        let original = price_batch(1);
        let mut copied = price_batch(2);
        copied.entry.date = date("2026-10-13");
        copied.entry.retrieved_at = observed();
        for file in &mut copied.entry.files {
            file.copied_from = Some(original.reference(&file.file_name));
        }
        let fixture = Fixture::new(&[original, copied]);
        let snapshot = fixture.scan();
        let scopes = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        let selected = snapshot
            .batches()
            .iter()
            .find(|batch| batch.batch_id() == BatchId::from_uuid(Uuid::from_u128(2)))
            .unwrap();
        let mut bars = Vec::new();
        let mut references = Vec::new();
        for file in selected.files() {
            let r = SourceFileReference {
                provider: "kis".into(),
                market: "kr".into(),
                batch_id: selected.batch_id(),
                file_name: file.file_name().into(),
                content_hash: file.content_hash().clone(),
                retrieved_at: selected.retrieved_at(),
            };
            let resolved = resolve_original(&scopes, &r).unwrap();
            if file.kind() == ResponseKind::Bars {
                bars.push(resolved)
            } else {
                references.push(resolved)
            }
        }
        let result = reconstruct_prices(date(DAY), &bars, &references).unwrap();
        assert!(
            result
                .bar_sources()
                .iter()
                .all(|file| file.copied_from.len() == 1 && file.batch_date == date("2026-10-13"))
        );
        assert_eq!(
            reconstruct_prices(date("2026-10-13"), &bars, &references).err(),
            Some(E::InvalidEvidence)
        );
    }

    #[test]
    fn s4_reconstruction_is_read_only_and_keeps_snapshot_lock() {
        let fixture = Fixture::new(&[price_batch(1)]);
        let before = fixture.fingerprint();
        let snapshot = fixture.scan();
        let result = prices(&snapshot).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(
                fixture
                    .root
                    .join("raw/manifests/provider=kis/market=kr/commit.lock"),
            )
            .unwrap();
        assert_eq!(
            FileExt::try_lock_exclusive(&lock).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(fixture.fingerprint(), before);
        drop(result);
        drop(snapshot);
        FileExt::try_lock_exclusive(&lock).unwrap();
        FileExt::unlock(&lock).unwrap();
        assert_eq!(fixture.fingerprint(), before);
    }

    #[test]
    fn s4_complete_seven_covers_weekend_by_request_not_partition() {
        let mut batch = action_batch(1, "2026-10-10", DAY, false);
        batch.entry.date = date("2026-01-01");
        let fixture = Fixture::new(&[batch]);
        let before = fixture.fingerprint();
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-09").unwrap();
        assert_eq!(result.witnesses().len(), 7);
        assert_eq!(result.selections().len(), 3 * 7 * 11);
        assert!(result.document().rows().is_empty());
        assert_eq!(result.document().descriptor().source_files.len(), 7);
        assert!(
            result
                .witnesses()
                .iter()
                .all(|chain| chain.range_start == date("2026-10-10")
                    && chain.pages[0].batch_date == date("2026-01-01"))
        );
        assert_eq!(fixture.fingerprint(), before);
    }

    #[test]
    fn s4_missing_civil_day_is_a_gap_and_complete_bundles_can_join() {
        assert_eq!(
            action_failure(&[action_batch(1, DAY, DAY, false)], "2026-10-09"),
            E::SourceCoverageGap
        );
        let batches = [
            action_batch(1, "2026-10-10", "2026-10-10", false),
            action_batch(2, "2026-10-11", "2026-10-11", false),
            action_batch(3, DAY, DAY, false),
        ];
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-09").unwrap();
        assert_eq!(result.witnesses().len(), 21);
        assert_eq!(result.selections().len(), 231);
    }

    #[test]
    fn s4_partial_bundles_cannot_be_combined_into_seven() {
        let mut first = action_batch(1, "2026-10-10", DAY, false);
        first.entry.files.pop();
        let mut second = action_batch(2, "2026-10-10", DAY, false);
        second
            .entry
            .files
            .retain(|file| file.file_name.contains("capital-decrease"));
        assert_eq!(
            action_failure(&[first, second], "2026-10-09"),
            E::SourceCoverageGap
        );
        let mut fixed = action_batch(1, "2026-10-10", DAY, true);
        fixed.entry.files.pop();
        assert_eq!(action_failure(&[fixed], "2026-10-09"), E::SourceCoverageGap);
    }

    #[test]
    fn s4_fixed_seventy_seven_and_whole_market_are_all_witnesses() {
        let mut whole = action_batch(1, "2026-10-10", DAY, false);
        let mut fixed = action_batch(2, "2026-10-10", DAY, true);
        let row = cash_row("069500", DAY);
        put_rows(
            &mut whole,
            "corporate-actions-dividend",
            None,
            json!([row.clone(), cash_row("11138K", DAY)]),
        );
        put_rows(
            &mut fixed,
            "corporate-actions-dividend",
            Some("069500"),
            json!([row]),
        );
        let fixture = Fixture::new(&[whole, fixed]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-09").unwrap();
        assert_eq!(result.witnesses().len(), 84);
        assert_eq!(result.document().descriptor().source_files.len(), 84);
        assert_eq!(result.document().rows().len(), 1);
        assert!(
            result
                .selections()
                .iter()
                .all(|cell| result.witnesses()[cell.witness_index].scope == Scope::Etf11Symbol)
        );
        assert_eq!(
            result.document().rows()[0]["source_file"]["batch_id"],
            json!(BatchId::from_uuid(Uuid::from_u128(2)))
        );
    }

    #[test]
    fn s4_hangul_stock_type_is_preserved_but_cannot_bypass_noncash_rejection() {
        for stock_rate in ["0", "1"] {
            let mut batch = action_batch(1, "2026-10-10", DAY, false);
            let mut row = cash_row("069500", DAY);
            row["stk_kind"] = json!("주식");
            row["stk_divi_rate"] = json!(stock_rate);
            put_rows(&mut batch, "corporate-actions-dividend", None, json!([row]));
            let fixture = Fixture::new(&[batch]);
            let snapshot = fixture.scan();
            let result = actions(&snapshot, "2026-10-09");
            if stock_rate == "0" {
                assert_eq!(
                    result.unwrap().document().rows()[0]["stock_kind"],
                    json!("주식")
                );
            } else {
                assert_eq!(result.err().unwrap(), E::NonCashAction);
            }
        }
    }

    #[test]
    fn s4_cash_metadata_decimals_empty_codes_and_future_payment_are_preserved() {
        let mut batch = action_batch(1, "2026-10-10", DAY, false);
        put_rows(
            &mut batch,
            "corporate-actions-dividend",
            None,
            cash_row("069500", "2026-10-11"),
        );
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-09").unwrap();
        let row = &result.document().rows()[0];
        assert_eq!(row["record_date"], json!("2026-10-11"));
        assert_eq!(row["dividend_kind"], json!(""));
        assert_eq!(row["face_value"], json!("1000"));
        assert_eq!(row["cash_dividend_amount"], json!("12.34"));
        assert_eq!(row["cash_dividend_rate"], json!("0.1234"));
        assert_eq!(row["stock_dividend_rate"], json!("0"));
        assert_eq!(row["cash_payment_date"], json!("2026-10-19"));
        assert_eq!(row["stock_payment_date"], Value::Null);
        assert_eq!(row["odd_lot_payment_date"], Value::Null);
        assert_eq!(row["stock_kind"], json!(""));
        assert_eq!(row["high_dividend_flag"], json!(""));
        let witness = result
            .witnesses()
            .iter()
            .find(|chain| chain.logical_class == Class::Dividend)
            .unwrap();
        assert_eq!(
            witness.cash_dividend_rows_sha256,
            ContentHash::from_bytes(
                &daily_cash_dividend_rows_preimage_v1(&witness.cash_dividends).unwrap()
            )
        );
    }

    #[test]
    fn s4_every_noncash_class_blocks_target_etf_and_non_etf_is_validated() {
        for (index, ActionFixtureSpec { label, .. }) in ACTIONS.iter().enumerate() {
            let row = match index {
                0..=2 => {
                    json!({"sht_cd":"069500","record_date":"20261012","right_dt":"20261013","fix_rate":"1"})
                }
                3 => {
                    let mut row = cash_row("069500", DAY);
                    row["stk_divi_rate"] = json!("1");
                    row
                }
                4 => {
                    json!({"sht_cd":"069500","record_date":"20261012","list_dt":"","merge_rate":"1"})
                }
                5 => {
                    json!({"sht_cd":"069500","record_date":"20261012","list_dt":"","inter_bf_face_amt":"500","inter_af_face_amt":"1000"})
                }
                _ => {
                    json!({"sht_cd":"069500","record_date":"20261012","list_dt":"","reduce_cap_rate":"1"})
                }
            };
            let mut blocked = action_batch(1, DAY, DAY, false);
            put_rows(&mut blocked, label, None, json!([row.clone()]));
            assert_eq!(action_failure(&[blocked], "2026-10-11"), E::NonCashAction);
            let mut non_etf = row;
            non_etf["sht_cd"] = json!("11138K");
            let mut allowed = action_batch(1, DAY, DAY, false);
            put_rows(&mut allowed, label, None, json!([non_etf]));
            let fixture = Fixture::new(&[allowed]);
            let snapshot = fixture.scan();
            assert!(
                actions(&snapshot, "2026-10-11")
                    .unwrap()
                    .document()
                    .rows()
                    .is_empty()
            );
            let mut malformed = action_batch(1, DAY, DAY, false);
            put_rows(
                &mut malformed,
                label,
                None,
                json!([{"sht_cd":"11138K","record_date":"20261012"}]),
            );
            assert_eq!(
                action_failure(&[malformed], "2026-10-11"),
                E::InvalidActions
            );
        }
    }

    #[test]
    fn s4_equivalent_overlap_compares_complete_rows_not_array_order() {
        let mut first = action_batch(1, "2026-10-10", DAY, false);
        let mut second = action_batch(2, "2026-10-10", DAY, false);
        let a = cash_row("069500", DAY);
        let mut b = cash_row("069500", DAY);
        b["divi_kind"] = json!("SECOND");
        put_rows(
            &mut first,
            "corporate-actions-dividend",
            None,
            json!([a.clone(), b.clone()]),
        );
        put_rows(
            &mut second,
            "corporate-actions-dividend",
            None,
            json!([b, a]),
        );
        let fixture = Fixture::new(&[first, second]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-09").unwrap();
        assert_eq!(result.witnesses().len(), 14);
        assert_eq!(result.document().rows().len(), 2);
        let dividend_chains: Vec<_> = result
            .witnesses()
            .iter()
            .filter(|chain| chain.logical_class == Class::Dividend)
            .collect();
        let chosen = dividend_chains
            .iter()
            .min_by_key(|chain| {
                (
                    chain.pages[0].manifest_line_sha256.to_string(),
                    chain.pages[0].batch_id.to_string(),
                )
            })
            .unwrap()
            .pages[0]
            .batch_id;
        assert!(
            result
                .document()
                .rows()
                .iter()
                .all(|row| row["source_file"]["batch_id"] == json!(chosen))
        );
    }

    #[test]
    fn s4_overlap_unknown_members_nested_arrays_and_scalar_types_must_agree() {
        for (left, right) in [
            (json!({"a":[1,2]}), json!({"a":[2,1]})),
            (json!({"a":1}), json!({"a":"1"})),
            (json!({"a":1}), json!({"a":2})),
        ] {
            let mut first = action_batch(1, DAY, DAY, false);
            let mut second = action_batch(2, DAY, DAY, false);
            let mut a = cash_row("069500", DAY);
            let mut b = a.clone();
            a["unmapped"] = left;
            b["unmapped"] = right;
            put_rows(&mut first, "corporate-actions-dividend", None, json!([a]));
            put_rows(&mut second, "corporate-actions-dividend", None, json!([b]));
            assert_eq!(
                action_failure(&[first, second], "2026-10-11"),
                E::SourceConflict
            );
        }
    }

    #[test]
    fn s4_empty_and_positive_overlap_conflict_in_either_input_order() {
        for reverse in [false, true] {
            let first = action_batch(1, DAY, DAY, false);
            let mut second = action_batch(2, DAY, DAY, false);
            put_rows(
                &mut second,
                "corporate-actions-dividend",
                None,
                json!([cash_row("069500", DAY)]),
            );
            let mut batches = vec![first, second];
            if reverse {
                batches.reverse();
            }
            assert_eq!(action_failure(&batches, "2026-10-11"), E::SourceConflict);
        }
    }

    #[test]
    fn s4_duplicate_events_within_or_across_pages_fail_without_collapsing() {
        for across in [false, true] {
            let mut batch = action_batch(1, DAY, DAY, false);
            let row = cash_row("069500", DAY);
            put_rows(
                &mut batch,
                "corporate-actions-dividend",
                None,
                json!([row.clone()]),
            );
            if across {
                append_page(
                    &mut batch,
                    "corporate-actions-dividend",
                    None,
                    2,
                    json!([row]),
                );
            } else {
                put_rows(
                    &mut batch,
                    "corporate-actions-dividend",
                    None,
                    json!([row.clone(), row]),
                );
            }
            assert_eq!(action_failure(&[batch], "2026-10-11"), E::SourceConflict);
        }
        let mut batch = action_batch(1, DAY, DAY, false);
        let row = cash_row("11138K", DAY);
        put_rows(
            &mut batch,
            "corporate-actions-dividend",
            None,
            json!([row.clone(), row]),
        );
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::SourceConflict);
    }

    #[test]
    fn s4_page_order_and_ten_page_boundary_use_terminal_original_metadata() {
        let mut batch = action_batch(1, DAY, DAY, false);
        for page in 2..=10 {
            append_page(
                &mut batch,
                "corporate-actions-dividend",
                None,
                page,
                json!([]),
            );
        }
        batch.entry.files.reverse();
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-11").unwrap();
        let chain = result
            .witnesses()
            .iter()
            .find(|chain| chain.logical_class == Class::Dividend)
            .unwrap();
        assert_eq!(chain.pages.len(), 10);
        assert!(chain.pages[0].file_name.ends_with("01.json"));
        assert!(chain.pages[9].file_name.ends_with("10.json"));
        let mut too_many = action_batch(1, DAY, DAY, false);
        for page in 2..=11 {
            append_page(
                &mut too_many,
                "corporate-actions-dividend",
                None,
                page,
                json!([]),
            );
        }
        assert_eq!(action_failure(&[too_many], "2026-10-11"), E::InvalidActions);
    }

    #[test]
    fn s4_missing_repeated_wrong_continuation_or_unterminated_pages_fail() {
        for case in 0..5 {
            let mut batch = action_batch(1, DAY, DAY, false);
            append_page(&mut batch, "corporate-actions-dividend", None, 2, json!([]));
            let first = action_name("corporate-actions-dividend", None, 1);
            let second = action_name("corporate-actions-dividend", None, 2);
            match case {
                0 => batch.entry.files.retain(|file| file.file_name != first),
                1 => {
                    let bytes = batch.bodies[&first].clone();
                    batch.body(&second, bytes);
                }
                2 => {
                    batch
                        .entry
                        .files
                        .iter_mut()
                        .find(|file| file.file_name == second)
                        .unwrap()
                        .request
                        .headers
                        .iter_mut()
                        .find(|(key, _)| key == "tr_cont")
                        .unwrap()
                        .1 = String::new()
                }
                3 => {
                    batch
                        .entry
                        .files
                        .iter_mut()
                        .find(|file| file.file_name == first)
                        .unwrap()
                        .response_continuation = Some("F".into())
                }
                _ => {
                    batch
                        .entry
                        .files
                        .iter_mut()
                        .find(|file| file.file_name == second)
                        .unwrap()
                        .response_continuation = Some("M".into())
                }
            }
            assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        }
    }

    #[test]
    fn s4_action_request_matrix_range_scope_and_returned_alias_are_exact() {
        for (key, value) in [
            ("CTS", "unexpected"),
            ("F_DT", "20261011"),
            ("T_DT", "20261013"),
            ("SHT_CD", "069500"),
        ] {
            let mut batch = action_batch(1, DAY, DAY, false);
            batch.entry.files[0]
                .request
                .query
                .iter_mut()
                .find(|(name, _)| name == key)
                .unwrap()
                .1 = value.into();
            assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        }
        let mut batch = action_batch(1, DAY, DAY, false);
        batch.entry.files[0]
            .request
            .query
            .push(("EXTRA".into(), "1".into()));
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        let mut batch = action_batch(1, DAY, DAY, true);
        let mut row = cash_row("069500", DAY);
        row["STCK_SHRN_ISCD"] = json!("102110");
        put_rows(
            &mut batch,
            "corporate-actions-dividend",
            Some("069500"),
            json!([row]),
        );
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
    }

    #[test]
    fn s4_out_of_range_rows_fail_and_valid_outside_extension_rows_are_not_emitted() {
        let mut batch = action_batch(1, DAY, DAY, false);
        put_rows(
            &mut batch,
            "corporate-actions-dividend",
            None,
            json!([cash_row("11138K", "2026-10-11")]),
        );
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        let mut batch = action_batch(1, "2026-10-09", "2026-10-13", false);
        put_rows(
            &mut batch,
            "corporate-actions-dividend",
            None,
            json!([
                cash_row("069500", "2026-10-09"),
                cash_row("069500", DAY),
                cash_row("069500", "2026-10-13")
            ]),
        );
        let fixture = Fixture::new(&[batch]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-11").unwrap();
        assert_eq!(result.document().rows().len(), 1);
        assert_eq!(result.document().rows()[0]["record_date"], json!(DAY));
    }

    #[test]
    fn s4_cash_classification_checks_every_field_before_exclusion() {
        for (field, value) in [
            ("face_val", json!("-1")),
            ("per_sto_divi_amt", json!("NaN")),
            ("divi_rate", json!(null)),
            ("stk_divi_rate", json!("-1")),
            ("divi_pay_dt", json!("20260230")),
            ("stk_div_pay_dt", json!("20261020")),
            ("odd_pay_dt", json!("20261020")),
            ("divi_kind", json!(3)),
            ("stk_kind", json!(null)),
            ("high_divi_gb", json!(false)),
        ] {
            let mut batch = action_batch(1, DAY, DAY, false);
            let mut row = cash_row("11138K", DAY);
            row[field] = value;
            put_rows(&mut batch, "corporate-actions-dividend", None, json!([row]));
            assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        }
    }

    #[test]
    fn s4_action_provider_json_and_response_shape_fail_closed() {
        for bytes in [
            br#"{"rt_cd":"0","output1":[],"output1":oops}"#.as_slice(),
            br#"{"rt_cd":"0","output1":[]} {}"#.as_slice(),
        ] {
            let mut batch = action_batch(1, DAY, DAY, false);
            batch.body(
                &action_name("corporate-actions-dividend", None, 1),
                bytes.to_vec(),
            );
            assert_eq!(
                action_failure(&[batch], "2026-10-11"),
                E::InvalidProviderJson
            );
        }
        let mut batch = action_batch(1, DAY, DAY, false);
        put_rows(&mut batch, "corporate-actions-dividend", None, json!(null));
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::InvalidActions);
        let mut batch = action_batch(1, DAY, DAY, false);
        batch.edit_body(
            &action_name("corporate-actions-dividend", None, 1),
            |body| body["rt_cd"] = json!(1),
        );
        assert_eq!(action_failure(&[batch], "2026-10-11"), E::ProviderResponse);
    }

    #[test]
    fn s4_action_copy_retains_original_ordinals_and_compares_both_witnesses() {
        let original = action_batch(1, DAY, DAY, false);
        let mut copied = action_batch(2, DAY, DAY, false);
        copied.entry.date = date("2026-10-13");
        copied.entry.retrieved_at = observed();
        for file in &mut copied.entry.files {
            file.copied_from = Some(original.reference(&file.file_name));
            let bytes = copied.bodies.remove(&file.file_name).unwrap();
            file.file_name = format!("copy-{}", file.file_name);
            copied.bodies.insert(file.file_name.clone(), bytes);
        }
        let fixture = Fixture::new(&[original, copied]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-11").unwrap();
        assert_eq!(result.witnesses().len(), 14);
        assert_eq!(result.document().descriptor().source_files.len(), 14);
        assert_eq!(
            result
                .witnesses()
                .iter()
                .filter(|chain| chain.pages[0].copied_from.len() == 1)
                .count(),
            7
        );
    }

    #[test]
    fn s4_action_interval_bounds_and_unrelated_ranges_do_not_create_coverage() {
        let fixture = Fixture::new(&[
            action_batch(1, DAY, DAY, false),
            action_batch(2, "2026-01-01", "2026-01-02", false),
        ]);
        let snapshot = fixture.scan();
        let context = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        assert_eq!(
            reconstruct_actions(&context, date(DAY), date(DAY)).err(),
            Some(E::InvalidContext)
        );
        assert_eq!(
            reconstruct_actions(&context, date("2025-01-01"), date(DAY)).err(),
            Some(E::LimitExceeded)
        );
        assert_eq!(
            actions(&snapshot, "2026-10-11").unwrap().witnesses().len(),
            7
        );
        let boundary_fixture = Fixture::new(&[action_batch(3, "2025-10-08", DAY, false)]);
        let boundary_snapshot = boundary_fixture.scan();
        assert_eq!(
            actions(&boundary_snapshot, "2025-10-07")
                .unwrap()
                .selections()
                .len(),
            370 * 77
        );
        assert_eq!(
            actions(&boundary_snapshot, "2025-10-06").err(),
            Some(E::LimitExceeded)
        );
    }

    #[test]
    fn s4_action_document_exact_bytes_and_all_witness_source_commitments() {
        let fixture = Fixture::new(&[action_batch(1, DAY, DAY, false)]);
        let snapshot = fixture.scan();
        let result = actions(&snapshot, "2026-10-11").unwrap();
        let document = result.document();
        let source = DailyNormalizedSourceCommitmentV1Wire {
            schema: "owner-beta-daily-normalized-source-v1".into(),
            kind: document.descriptor().kind,
            session_date: date(DAY),
            source_files: document.descriptor().source_files.clone(),
        };
        let hash = ContentHash::from_bytes(&daily_normalized_source_preimage_v1(&source).unwrap());
        let expected = format!(
            "{{\"kind\":\"corporate_actions\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"{DAY}\",\"source_evidence_sha256\":\"{hash}\"}}"
        );
        assert_eq!(document.bytes(), expected.as_bytes());
        assert_eq!(document.descriptor().source_evidence_sha256, hash);
        assert_eq!(
            document.descriptor().sha256,
            ContentHash::from_bytes(expected.as_bytes())
        );
    }

    #[test]
    fn s4_price_document_exact_encoding_preserves_legacy_numeric_mapper() {
        let fixture = Fixture::new(&[price_batch(1)]);
        let snapshot = fixture.scan();
        let result = prices(&snapshot).unwrap();
        let mut symbols = KR_ETF_CORE_SYMBOLS;
        symbols.sort_unstable();
        let bars=symbols.iter().map(|symbol|format!("{{\"close\":110,\"date\":\"{DAY}\",\"high\":120,\"instrument\":\"{symbol}.KRX\",\"low\":90.0,\"open\":100.0,\"value\":22000,\"volume\":200}}")).collect::<Vec<_>>().join(",");
        let references = symbols
            .iter()
            .map(|symbol| {
                format!("{{\"instrument\":\"{symbol}.KRX\",\"provider_symbol\":\"{symbol}\"}}")
            })
            .collect::<Vec<_>>()
            .join(",");
        for (kind, document, rows) in [
            ("bars", result.bars(), bars),
            ("reference", result.reference(), references),
        ] {
            let expected = format!(
                "{{\"kind\":\"{kind}\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[{rows}],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"{DAY}\",\"source_evidence_sha256\":\"{}\"}}",
                document.descriptor().source_evidence_sha256
            );
            assert_eq!(document.bytes(), expected.as_bytes());
        }
    }

    #[test]
    fn s4_action_selection_is_independent_of_manifest_iteration_order() {
        let mut first = action_batch(1, "2026-10-10", DAY, false);
        let mut second = action_batch(2, "2026-10-10", DAY, false);
        let row = cash_row("069500", DAY);
        put_rows(
            &mut first,
            "corporate-actions-dividend",
            None,
            json!([row.clone()]),
        );
        put_rows(
            &mut second,
            "corporate-actions-dividend",
            None,
            json!([row]),
        );
        let mut batches = vec![first, second];
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let original = actions(&snapshot, "2026-10-09").unwrap();
        batches.reverse();
        let reordered_fixture = Fixture::new(&batches);
        let reordered_snapshot = reordered_fixture.scan();
        let reordered = actions(&reordered_snapshot, "2026-10-09").unwrap();
        assert_eq!(original.document().bytes(), reordered.document().bytes());
        assert_eq!(original.selections(), reordered.selections());
    }
}
