//! Real committed Raw and the shared refresh branch; no protected-base authority.

use super::*;
use crate::daily_price_only::admission::DailyAdmissionVerificationError as AdmissionError;
use crate::daily_price_only::frontier::select_reconstructed_sealed_session;
use crate::daily_price_only::source::{
    ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, ScanLimitValues, ScanLimits,
};
use crate::storage::ManifestEntry;
use domain::BatchId;
use std::cell::Cell;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

const ENTITLEMENT: &str = "fixture-only:synthetic-wholeflow-extension-not-market-receipt";

#[derive(Clone, Copy)]
enum Mutation {
    None,
    ChangedPrice,
    CalendarConflict,
    CalendarGap,
    CalendarTargetMissing,
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn write(root: &Path, path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut parent = path.parent().unwrap();
    while parent != root {
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
        parent = parent.parent().unwrap();
    }
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn fixture(prior_id: u128, mutation: Mutation) -> Fixture {
    // Reuse the existing pure two-session Raw fixture, not a new market engine.
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
    assert!(output.status.success() && output.stderr.is_empty());
    let raw: BTreeMap<String, String> = serde_json::from_slice(&output.stdout).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    let mut manifests: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (name, metadata) in raw.iter().filter(|(name, _)| name.ends_with("/batch.json")) {
        let old_folder = name.strip_suffix("batch.json").unwrap();
        let mut entry: ManifestEntry = serde_json::from_str(metadata).unwrap();
        let prior = entry.provider == "kis-calendar";
        if prior {
            entry.batch_id = BatchId::from_uuid(Uuid::from_u128(prior_id));
            entry.date = TradingDate::parse("2026-08-31").unwrap();
        } else if entry.date == TradingDate::parse("2026-10-18").unwrap() {
            entry.batch_id = BatchId::from_uuid(Uuid::from_u128(1000 + prior_id));
        }
        let folder = root.join(format!(
            "raw/provider={}/market=kr/date={}/batch={}",
            entry.provider, entry.date, entry.batch_id
        ));
        for file in &mut entry.files {
            let mut body: Value =
                serde_json::from_str(&raw[&format!("{old_folder}{}", file.file_name)]).unwrap();
            if file.kind == ResponseKind::Calendar {
                if prior {
                    for (key, value) in &mut file.request.query {
                        if key == "BASS_DT" {
                            *value = "20260831".to_owned();
                        }
                    }
                }
                let rows = body["output"].as_array_mut().unwrap();
                match mutation {
                    Mutation::CalendarConflict if prior => {
                        rows.last_mut().unwrap()["opnd_yn"] = Value::String("N".into());
                    }
                    Mutation::CalendarGap => rows.retain(|row| row["bass_dt"] != "20261001"),
                    Mutation::CalendarTargetMissing if prior => {
                        rows.retain(|row| row["bass_dt"] != "20260831");
                    }
                    _ => {}
                }
            } else if file.kind == ResponseKind::Bars
                && entry.date == TradingDate::parse("2026-10-18").unwrap()
                && matches!(mutation, Mutation::ChangedPrice)
            {
                body["output2"][0]["stck_clpr"] = Value::String("112".into());
                body["output2"][0]["acml_tr_pbmn"] = Value::String("22400".into());
            }
            let bytes = serde_json::to_vec(&body).unwrap();
            file.content_hash = ContentHash::from_bytes(&bytes);
            file.size_bytes = bytes.len() as u64;
            write(&root, &folder.join(&file.file_name), &bytes);
        }
        let bytes = serde_json::to_vec(&entry).unwrap();
        write(&root, &folder.join("batch.json"), &bytes);
        let manifest = manifests.entry(entry.provider).or_default();
        manifest.extend(bytes);
        manifest.push(b'\n');
    }
    for (provider, bytes) in manifests {
        let folder = root.join(format!("raw/manifests/provider={provider}/market=kr"));
        write(&root, &folder.join("commit.lock"), b"");
        write(&root, &folder.join("manifest.jsonl"), &bytes);
    }
    Fixture { _temp: temp, root }
}

impl Fixture {
    fn read(&self) -> [CommittedRawSnapshot; 2] {
        let uid = fs::metadata(self.root.join("raw")).unwrap().uid();
        ["kis", "kis-calendar"].map(|provider| {
            CommittedRawReader::new(&self.root)
                .snapshot(
                    ApprovedRawScope::new(provider, "kr", uid).unwrap(),
                    ScanLimits::new(ScanLimitValues {
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
                    })
                    .unwrap(),
                )
                .unwrap()
        })
    }
}

fn request() -> SessionSourceRequest {
    SessionSourceRequest {
        owner: Uuid::from_u128(1),
        policy_sha256: ContentHash::from_bytes(b"fixture-policy"),
        previous_frontier: TradingDate::parse("2026-08-31").unwrap(),
        session_date: TradingDate::parse("2026-10-18").unwrap(),
    }
}

fn context(scopes: &[CommittedRawSnapshot; 2]) -> RawEvidenceSnapshots<'_> {
    RawEvidenceSnapshots::new(
        &[&scopes[0], &scopes[1]],
        ENTITLEMENT,
        domain::UtcTimestamp::parse_rfc3339("2026-10-19T01:24:01Z").unwrap(),
    )
    .unwrap()
}

#[test]
fn current_calendar_raw_shared_compatibility_preserves_exact_previous_bytes() {
    let mut exercised_legacy = false;
    for id in 3..=32 {
        let fixture = fixture(id, Mutation::None);
        let scopes = fixture.read();
        let snapshots = context(&scopes);
        let preferred = discover_session_sources(&snapshots, &request()).unwrap();
        let legacy = discover_session_sources_with_selection(
            &snapshots,
            &request(),
            CalendarSelection::LegacyHash,
        )
        .unwrap();
        assert_eq!(preferred.calendars().len(), 2);
        assert_eq!(
            preferred.observation().files(),
            legacy.observation().files()
        );
        let exact = preferred.session().clone();
        let selected = select_reconstructed_sealed_session(&exact, Ok(preferred), || {
            panic!("exact preferred reconstruction must not invoke legacy")
        })
        .unwrap();
        assert_eq!(selected.session(), &exact);
        if legacy.session() == &exact {
            continue;
        }
        let previous_bytes = serde_json::to_vec(legacy.session()).unwrap();
        let path = fixture.root.join("previous-session.json");
        fs::write(&path, &previous_bytes).unwrap();
        let previous: DailySessionEvidenceV1Wire =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let calls = Cell::new(0);
        let matched = select_reconstructed_sealed_session(
            &previous,
            discover_session_sources(&snapshots, &request()).map_err(AdmissionError::Evidence),
            || {
                calls.set(calls.get() + 1);
                discover_session_sources_with_selection(
                    &snapshots,
                    &request(),
                    CalendarSelection::LegacyHash,
                )
                .map_err(AdmissionError::Evidence)
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(
            serde_json::to_vec(matched.session()).unwrap(),
            previous_bytes
        );
        assert_eq!(matched.evidence_preimage(), legacy.evidence_preimage());
        super::super::candidate::validate_documents(
            &previous.normalized_documents,
            matched
                .documents()
                .iter()
                .map(|document| (document.descriptor(), document.bytes())),
        )
        .unwrap();

        let changed = self::fixture(id, Mutation::ChangedPrice);
        let changed_scopes = changed.read();
        let changed_snapshots = context(&changed_scopes);
        let refused = select_reconstructed_sealed_session(
            &previous,
            discover_session_sources(&changed_snapshots, &request())
                .map_err(AdmissionError::Evidence),
            || {
                discover_session_sources_with_selection(
                    &changed_snapshots,
                    &request(),
                    CalendarSelection::LegacyHash,
                )
                .map_err(AdmissionError::Evidence)
            },
        );
        assert!(matches!(refused, Err(AdmissionError::InvalidCandidate)));

        for (mutation, expected) in [
            (Mutation::CalendarConflict, Error::CalendarConflict),
            (Mutation::CalendarGap, Error::CalendarCoverageGap),
            (
                Mutation::CalendarTargetMissing,
                Error::CalendarTargetMissing,
            ),
        ] {
            let invalid = self::fixture(id, mutation);
            let invalid_scopes = invalid.read();
            let invalid_snapshots = context(&invalid_scopes);
            let refused = select_reconstructed_sealed_session(
                &previous,
                discover_session_sources(&invalid_snapshots, &request())
                    .map_err(AdmissionError::Evidence),
                || panic!("a preferred discovery error must never invoke legacy"),
            );
            assert!(matches!(refused, Err(AdmissionError::Evidence(error)) if error == expected));
        }
        exercised_legacy = true;
        break;
    }
    assert!(exercised_legacy);
}
