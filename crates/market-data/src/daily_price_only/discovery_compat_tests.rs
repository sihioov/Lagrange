//! Real committed Raw and the shared refresh branch; no protected-base authority.

use super::*;
use crate::HistoricalPriceOnlyBar;
use crate::daily_price_only::admission::DailyAdmissionVerificationError as AdmissionError;
use crate::daily_price_only::frontier::{
    appended_sealed_observation, select_reconstructed_sealed_session,
};
use crate::daily_price_only::observation::{
    DailyBlockCode, DailySourceKind, compare_source_observation,
};
use crate::daily_price_only::price_extension::{
    PricePrefixView, compare_observed_prefix, reconstruct_prefix_prices,
};
use crate::daily_price_only::source::{
    ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, ScanLimitValues, ScanLimits,
};
use crate::storage::ManifestEntry;
use domain::{BatchId, FixedPoint};
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

#[derive(Clone, Copy)]
enum ClosedDayBar {
    PreviousSession,
    ChangedPrice,
    UnknownDate,
    TradingDay,
    CalendarConflict,
    LaterRow,
    Empty,
    SelectedTargetMissing,
    UncoveredPriorRow,
    SplitCalendar,
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

fn fixture(prior_id: u128, mutation: Mutation, closed_bar: Option<ClosedDayBar>) -> Fixture {
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
                if let Some(closed_bar) = closed_bar {
                    match closed_bar {
                        ClosedDayBar::UnknownDate => {
                            rows.retain(|row| row["bass_dt"] != "20260830")
                        }
                        ClosedDayBar::SplitCalendar => rows.retain(|row| {
                            let date = row["bass_dt"].as_str().unwrap();
                            (date <= "20260831") == prior
                        }),
                        kind if matches!(kind, ClosedDayBar::TradingDay)
                            || (prior && matches!(kind, ClosedDayBar::CalendarConflict)) =>
                        {
                            let closed = rows
                                .iter_mut()
                                .find(|row| row["bass_dt"] == "20260830")
                                .unwrap();
                            closed["opnd_yn"] = Value::String("Y".into());
                        }
                        _ => {}
                    }
                }
            } else if file.kind == ResponseKind::Bars
                && entry.date == TradingDate::parse("2026-10-18").unwrap()
                && matches!(mutation, Mutation::ChangedPrice)
            {
                body["output2"][0]["stck_clpr"] = Value::String("112".into());
                body["output2"][0]["acml_tr_pbmn"] = Value::String("22400".into());
            } else if file.kind == ResponseKind::Bars
                && entry.date == day("2026-08-31")
                && matches!(closed_bar, Some(ClosedDayBar::SelectedTargetMissing))
            {
                body["output2"][0]["stck_bsop_date"] = Value::String("20260828".into());
            }
            let bytes = serde_json::to_vec(&body).unwrap();
            file.content_hash = ContentHash::from_bytes(&bytes);
            file.size_bytes = bytes.len() as u64;
            write(&root, &folder.join(&file.file_name), &bytes);
        }
        let bytes = serde_json::to_vec(&entry).unwrap();
        write(&root, &folder.join("batch.json"), &bytes);
        let manifest = manifests.entry(entry.provider.clone()).or_default();
        manifest.extend(bytes);
        manifest.push(b'\n');

        if let Some(closed_bar) = closed_bar.filter(|_| !prior && entry.date == day("2026-08-31")) {
            entry.date = day("2026-08-30");
            entry.batch_id = BatchId::from_uuid(Uuid::from_u128(2000 + prior_id));
            entry.files.retain(|file| file.kind == ResponseKind::Bars);
            let folder = root.join(format!(
                "raw/provider=kis/market=kr/date={}/batch={}",
                entry.date, entry.batch_id
            ));
            for file in &mut entry.files {
                for (key, value) in &mut file.request.query {
                    if key == "FID_INPUT_DATE_1" || key == "FID_INPUT_DATE_2" {
                        *value = "20260830".into();
                    }
                }
                let mut body: Value =
                    serde_json::from_str(&raw[&format!("{old_folder}{}", file.file_name)]).unwrap();
                let earlier = match closed_bar {
                    ClosedDayBar::LaterRow => "20260831",
                    ClosedDayBar::UncoveredPriorRow => "20260827",
                    _ => "20260828",
                };
                body["output2"][0]["stck_bsop_date"] = Value::String(earlier.into());
                match closed_bar {
                    ClosedDayBar::ChangedPrice => {
                        body["output2"][0]["stck_clpr"] = Value::String("109".into());
                    }
                    ClosedDayBar::Empty => body["output2"].as_array_mut().unwrap().clear(),
                    _ => {}
                }
                let bytes = serde_json::to_vec(&body).unwrap();
                file.content_hash = ContentHash::from_bytes(&bytes);
                file.size_bytes = bytes.len() as u64;
                write(&root, &folder.join(&file.file_name), &bytes);
            }
            let bytes = serde_json::to_vec(&entry).unwrap();
            write(&root, &folder.join("batch.json"), &bytes);
            manifest.extend(bytes);
            manifest.push(b'\n');
        }
    }
    for (provider, bytes) in manifests {
        let folder = root.join(format!("raw/manifests/provider={provider}/market=kr"));
        write(&root, &folder.join("commit.lock"), b"");
        write(&root, &folder.join("manifest.jsonl"), &bytes);
    }
    Fixture { _temp: temp, root }
}

fn day(value: &str) -> TradingDate {
    TradingDate::parse(value).unwrap()
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

impl Fixture {
    fn append_action_alternative(&self, id: u128, truncated: bool) {
        let manifest = self
            .root
            .join("raw/manifests/provider=kis/market=kr/manifest.jsonl");
        let records = fs::read_to_string(&manifest).unwrap();
        let mut entry: ManifestEntry =
            serde_json::from_str(records.lines().next().unwrap()).unwrap();
        assert_eq!(entry.date, day("2026-08-31"));
        entry.batch_id = BatchId::from_uuid(Uuid::from_u128(id));
        entry.retrieved_at = domain::UtcTimestamp::parse_rfc3339("2026-10-19T01:24:01Z").unwrap();
        entry
            .files
            .retain(|leaf| leaf.kind == ResponseKind::CorporateActions);
        let folder = self.root.join(format!(
            "raw/provider=kis/market=kr/date={}/batch={}",
            entry.date, entry.batch_id
        ));
        let mut extra_page = None;
        for file in &mut entry.files {
            let mut body = serde_json::json!({"rt_cd":"0","output1":[]});
            if truncated && file.file_name == "corporate-actions-bonus-page-01.json" {
                file.response_continuation = Some("M".into());
                let mut terminal = file.clone();
                terminal.file_name = "corporate-actions-bonus-page-02.json".into();
                terminal.response_continuation = Some("F".into());
                terminal
                    .request
                    .headers
                    .iter_mut()
                    .find(|(key, _)| key == "tr_cont")
                    .unwrap()
                    .1 = "N".into();
                let terminal_body =
                    serde_json::to_vec(&serde_json::json!({"rt_cd":"0","output1":[{}]})).unwrap();
                terminal.content_hash = ContentHash::from_bytes(&terminal_body);
                terminal.size_bytes = terminal_body.len() as u64;
                write(
                    &self.root,
                    &folder.join(&terminal.file_name),
                    &terminal_body,
                );
                extra_page = Some(terminal);
            }
            if truncated && file.file_name == "corporate-actions-dividend-page-01.json" {
                file.response_continuation = None;
                body["output1"] = Value::Array(vec![Value::Null; 100]);
            }
            let wire = serde_json::to_vec(&body).unwrap();
            file.content_hash = ContentHash::from_bytes(&wire);
            file.size_bytes = wire.len() as u64;
            write(&self.root, &folder.join(&file.file_name), &wire);
        }
        if truncated {
            entry
                .files
                .push(extra_page.expect("multipart truncated fixture"));
        }
        self.append_batch(&entry, &folder, &manifest, records);
    }

    fn append_price_alternative(
        &self,
        id: u128,
        source_date: TradingDate,
        partition_date: TradingDate,
        kind: ResponseKind,
    ) {
        let manifest = self
            .root
            .join("raw/manifests/provider=kis/market=kr/manifest.jsonl");
        let records = fs::read_to_string(&manifest).unwrap();
        let mut entry = records
            .lines()
            .map(|line| serde_json::from_str::<ManifestEntry>(line).unwrap())
            .find(|entry| entry.date == source_date)
            .unwrap();
        let source_folder = self.root.join(format!(
            "raw/provider=kis/market=kr/date={}/batch={}",
            entry.date, entry.batch_id
        ));
        entry.batch_id = BatchId::from_uuid(Uuid::from_u128(id));
        entry.date = partition_date;
        entry.retrieved_at = domain::UtcTimestamp::parse_rfc3339("2026-10-19T01:24:01Z").unwrap();
        let folder = self.root.join(format!(
            "raw/provider=kis/market=kr/date={}/batch={}",
            entry.date, entry.batch_id
        ));
        let file = entry
            .files
            .into_iter()
            .find(|file| file.kind == kind)
            .unwrap();
        let original = fs::read(source_folder.join(&file.file_name)).unwrap();
        write(&self.root, &folder.join(&file.file_name), &original);
        entry.files = vec![file];
        self.append_batch(&entry, &folder, &manifest, records);
    }

    fn append_batch(&self, entry: &ManifestEntry, folder: &Path, manifest: &Path, records: String) {
        let metadata = serde_json::to_vec(entry).unwrap();
        write(&self.root, &folder.join("batch.json"), &metadata);
        let mut bytes = records.into_bytes();
        bytes.extend(metadata);
        bytes.push(b'\n');
        write(&self.root, manifest, &bytes);
    }
}

#[test]
fn frontier_refresh_ignores_only_truncated_action_chains_and_still_detects_a_complete_append() {
    let request = first_session();
    let range = day("2026-08-29")..=request.session_date;
    let sealed_raw = fixture(3, Mutation::None, None);
    sealed_raw.append_action_alternative(4020, true);
    let original = sealed_raw.read();
    let snapshots = context(&original);
    let sealed = discover_session_sources(&snapshots, &request).unwrap();
    let expected = sealed.observation();
    let alternative = BatchId::from_uuid(Uuid::from_u128(4020));
    assert_eq!(
        expected
            .files()
            .iter()
            .filter(|file| file.batch_id == alternative)
            .count(),
        5,
        "both truncated chains are excluded while their complete siblings remain"
    );
    let observe = |snapshots: &RawEvidenceSnapshots<'_>| {
        let observed = appended_sealed_observation(
            request.owner,
            &request.policy_sha256,
            range.clone(),
            &BTreeSet::from([request.session_date]),
            expected,
            snapshots,
        )
        .unwrap();
        compare_source_observation(
            request.owner,
            *range.start(),
            *range.end(),
            expected,
            &observed,
        )
        .unwrap()
    };
    assert!(observe(&snapshots).is_none());

    let fresh_raw = fixture(3, Mutation::None, None);
    fresh_raw.append_action_alternative(4020, true);
    fresh_raw.append_action_alternative(4021, false);
    let fresh = fresh_raw.read();
    let event = observe(&context(&fresh)).expect("complete overlap must be attributed");
    assert_eq!(event.code(), DailyBlockCode::SealedSourceChanged);
    assert_eq!(event.kind(), DailySourceKind::Action);

    // Mutation keeps the recorded size and path. The retained descriptor must
    // still reject the wire bytes even when the page belongs to a complete
    // action alternative that was never the stable winner.
    let scope = &original[0];
    let path = sealed_raw.root.join(format!(
        "raw/provider=kis/market=kr/date=2026-08-31/batch={alternative}/corporate-actions-paidin-record-page-01.json"
    ));
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] = b' ';
    fs::write(path, bytes).unwrap();
    assert!(scope.revalidate().is_err());
}

#[test]
fn frontier_refresh_compares_positive_prices_for_all_sealed_trading_sessions() {
    use crate::daily_price_only::observation::canonical_source_observation;

    let first = first_session();
    let last = request();
    let range = day("2026-08-29")..=last.session_date;
    let sessions = BTreeSet::from([first.session_date, last.session_date]);
    let original = fixture(3, Mutation::None, Some(ClosedDayBar::PreviousSession));
    // A real closed-date reference identifies the instrument only.
    original.append_price_alternative(
        4000,
        first.session_date,
        day("2026-08-30"),
        ResponseKind::Reference,
    );
    let raws = original.read();
    let snapshots = context(&raws);
    let one = discover_session_sources(&snapshots, &first).unwrap();
    let two = discover_session_sources(&snapshots, &last).unwrap();
    let mut files = one.observation().files().to_vec();
    files.extend_from_slice(two.observation().files());
    let baseline = canonical_source_observation(&files).unwrap();
    let observe = |snapshots: &RawEvidenceSnapshots<'_>| {
        let all = appended_sealed_observation(
            first.owner,
            &first.policy_sha256,
            range.clone(),
            &sessions,
            &baseline,
            snapshots,
        )
        .unwrap();
        compare_source_observation(first.owner, *range.start(), *range.end(), &baseline, &all)
            .unwrap()
    };
    assert!(observe(&snapshots).is_none());

    for (session_date, kind, source_kind) in [
        (first.session_date, ResponseKind::Bars, DailySourceKind::Bar),
        (
            last.session_date,
            ResponseKind::Reference,
            DailySourceKind::Reference,
        ),
    ] {
        let more = fixture(3, Mutation::None, Some(ClosedDayBar::PreviousSession));
        more.append_price_alternative(
            4000,
            first.session_date,
            day("2026-08-30"),
            ResponseKind::Reference,
        );
        more.append_price_alternative(4001, session_date, session_date, kind);
        let snapshots = more.read();
        let event = observe(&context(&snapshots)).expect("true trading-session append");
        assert_eq!(
            (event.kind(), event.code()),
            (source_kind, DailyBlockCode::SealedSourceChanged)
        );
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

fn first_session() -> SessionSourceRequest {
    SessionSourceRequest {
        previous_frontier: day("2026-08-28"),
        session_date: day("2026-08-31"),
        ..request()
    }
}

fn previous_bars(session_date: TradingDate) -> Vec<HistoricalPriceOnlyBar> {
    let fixed = |value| FixedPoint::parse(value).unwrap();
    let [open, high, low, close, value] = ["100", "120", "90", "110", "22000"].map(fixed);
    let mut bars: Vec<_> = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| HistoricalPriceOnlyBar {
            instrument_id: InstrumentId::from_str(&format!("{symbol}.KRX")).unwrap(),
            session_date,
            raw_open: open,
            raw_high: high,
            raw_low: low,
            raw_close: close,
            raw_volume: 200,
            raw_trading_value: Some(value),
            adjusted_open: open,
            adjusted_high: high,
            adjusted_low: low,
            adjusted_close: close,
        })
        .collect();
    bars.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    bars
}

#[test]
fn closed_day_bars_are_kept_for_exact_prefix_comparison_only() {
    let request = first_session();
    let sessions = [request.previous_frontier];
    let prefix = previous_bars(request.previous_frontier);
    let view = || PricePrefixView {
        bars: &prefix,
        sessions: &sessions,
        policy_sha256: &request.policy_sha256,
    };
    let fixture = fixture(3, Mutation::None, Some(ClosedDayBar::PreviousSession));
    let snapshots = fixture.read();
    let context = context(&snapshots);
    let sources = discover_session_sources(&context, &request).unwrap();
    assert_eq!(sources.session().bars.len(), 11);
    assert!(
        sources
            .session()
            .bars
            .iter()
            .all(|file| file.batch_date == request.session_date)
    );
    assert_eq!(
        sources
            .observed_bars()
            .iter()
            .filter(|original| original.coverage() == BarRowCoverage::ClosedDayOverlap)
            .count(),
        11
    );
    let prices = reconstruct_prefix_prices(view(), &sources, &context).unwrap();
    assert_eq!(prices.inspected_rows, 33);
    assert_eq!(prices.compared_rows, 11);
    assert_eq!(prices.added_bars.len(), 11);
    assert_eq!(
        compare_observed_prefix(view(), &sources, &context).unwrap(),
        None
    );
}

#[test]
fn closed_day_proof_remains_available_when_the_later_session_uses_a_new_calendar() {
    let fixture = fixture(3, Mutation::None, Some(ClosedDayBar::SplitCalendar));
    let snapshots = fixture.read();
    let sources = discover_session_sources(&context(&snapshots), &request()).unwrap();
    assert_eq!(sources.calendars().len(), 1);
    assert_eq!(
        sources.calendars()[0].source_file().batch_date,
        day("2026-10-18")
    );
    assert!(sources.observed_bars().iter().any(|original| {
        original.source().batch_date == day("2026-08-30")
            && original.coverage() == BarRowCoverage::ClosedDayOverlap
    }));
}

#[test]
fn closed_day_bars_still_fail_on_a_different_or_uncovered_prefix_row() {
    let request = first_session();
    let sessions = [request.previous_frontier];
    let prefix = previous_bars(request.previous_frontier);
    let view = || PricePrefixView {
        bars: &prefix,
        sessions: &sessions,
        policy_sha256: &request.policy_sha256,
    };
    for (variant, expected) in [
        (ClosedDayBar::ChangedPrice, Error::SourceConflict),
        (ClosedDayBar::UncoveredPriorRow, Error::SourceCoverageGap),
    ] {
        let changed = fixture(3, Mutation::None, Some(variant));
        let snapshots = changed.read();
        let context = context(&snapshots);
        let sources = discover_session_sources(&context, &request).unwrap();
        assert_eq!(
            reconstruct_prefix_prices(view(), &sources, &context).err(),
            Some(expected)
        );
        let refresh = compare_observed_prefix(view(), &sources, &context);
        if matches!(variant, ClosedDayBar::ChangedPrice) {
            let witness = refresh.unwrap().unwrap();
            assert_eq!(witness.batch_date, day("2026-08-30"));
        } else {
            assert_eq!(refresh.err(), Some(Error::SourceCoverageGap));
        }
    }
}

#[test]
fn missing_target_still_requires_an_earlier_verified_closed_request_and_prior_rows() {
    for (variant, expected) in [
        (ClosedDayBar::UnknownDate, Error::SourceCoverageGap),
        (ClosedDayBar::TradingDay, Error::SourceCoverageGap),
        (ClosedDayBar::CalendarConflict, Error::CalendarConflict),
        (ClosedDayBar::LaterRow, Error::SourceCoverageGap),
        (ClosedDayBar::Empty, Error::SourceCoverageGap),
        (
            ClosedDayBar::SelectedTargetMissing,
            Error::SourceCoverageGap,
        ),
    ] {
        let fixture = fixture(3, Mutation::None, Some(variant));
        let snapshots = fixture.read();
        assert_eq!(
            discover_session_sources(&context(&snapshots), &first_session()).err(),
            Some(expected)
        );
    }
}

#[test]
fn current_calendar_raw_shared_compatibility_preserves_exact_previous_bytes() {
    let mut exercised_legacy = false;
    for id in 3..=32 {
        let fixture = fixture(id, Mutation::None, None);
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

        let changed = self::fixture(id, Mutation::ChangedPrice, None);
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
            let invalid = self::fixture(id, mutation, None);
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
