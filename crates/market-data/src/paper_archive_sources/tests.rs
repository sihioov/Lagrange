// Source-only Linux tests. Fixtures are copied from the adopted Raw reader shape;
// expected normalized JSON is assembled independently of the reconstruction.
use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use domain::BatchId;
use crate::contract::{FetchMode, RequestMetadata, ResponseKind};
use crate::daily_price_only::source::{ApprovedRawScope, CommittedRawReader, ScanLimitValues, ScanLimits};
use crate::daily_price_only::verify::{DailyVerificationError as E, ResolvedOriginal};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::storage::{FileEntry, ManifestEntry};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;
fn complete_discovery_batches() -> Vec<Batch> {
    let mut calendar = Batch::new(3, DAY);
    calendar.add(
        ResponseKind::Calendar, "calendar.json", "quotations/chk-holiday", "CTCA0903R",
        vec![("BASS_DT".into(), "20261012".into()), ("CTX_AREA_FK".into(), "".into()),
            ("CTX_AREA_NK".into(), "".into())],
        json!({"rt_cd":"0","output":[
            {"bass_dt":"20261010","opnd_yn":"N"},
            {"bass_dt":"20261011","opnd_yn":"N"},
            {"bass_dt":"20261012","opnd_yn":"Y"}
        ]}),
    );
    vec![price_batch(1), action_batch(2, "2026-10-10", DAY, false), calendar]
}

fn discovery_request() -> crate::daily_price_only::discovery::SessionSourceRequest {
    crate::daily_price_only::discovery::SessionSourceRequest {
        owner: Uuid::from_u128(9),
        policy_sha256: ContentHash::from_bytes(b"recorded-policy"),
        previous_frontier: date("2026-10-09"),
        session_date: date(DAY),
    }
}

#[test]
fn actual_complete_daily_discovery_reconstructs_every_document_kind() {
    let fixture = Fixture::new(&complete_discovery_batches());
    let before = fixture.fingerprint();
    let snapshot = fixture.scan();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let discovered = crate::daily_price_only::discovery::discover_session_sources(
        &raw.evidence, &discovery_request()).unwrap();
    assert_eq!(discovered.session().bars.len(), 11);
    assert_eq!(discovered.session().reference.len(), 11);
    assert_eq!(discovered.session().calendar.len(), 3);
    assert_eq!(discovered.session().actions.len(), 7);
    assert_eq!(discovered.documents().len(), 6);
    assert_eq!(snapshot.batches().len(), 3);
    raw.revalidate().unwrap();
    assert_eq!(before, fixture.fingerprint());
}

#[test]
fn historical_timestamp_alone_never_filters_later_retained_originals() {
    for conflicting in [false, true] {
        let mut batches = complete_discovery_batches();
        let mut later = price_batch(4);
        later.entry.retrieved_at = time("2026-10-13T08:00:00Z");
        if conflicting {
            later.edit_body("bar-069500.json", |value| {
                value["output2"][0]["stck_clpr"] = json!("111");
            });
        }
        batches.push(later);
        // One actual complete captured tree, never a fabricated prior snapshot.
        let fixture = Fixture::new(&batches);
        let before = fixture.fingerprint();
        let snapshot = fixture.scan();
        let old_time = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        assert!(matches!(
            crate::daily_price_only::discovery::discover_session_sources(
                &old_time, &discovery_request()),
            Err(E::FutureRetrieval)
        ));
        assert_eq!(snapshot.batches().len(), 4);
        snapshot.revalidate().unwrap();
        assert_eq!(before, fixture.fingerprint());
    }
}

#[test]
fn live_discovery_still_rejects_later_equal_or_conflicting_delivery() {
    for conflicting in [false, true] {
        let mut batches = complete_discovery_batches();
        let mut later = price_batch(4);
        later.entry.retrieved_at = time("2026-10-13T08:00:00Z");
        if conflicting {
            later.edit_body("bar-069500.json", |value| {
                value["output2"][0]["stck_clpr"] = json!("111");
            });
        }
        batches.push(later);
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let live = RawEvidenceSnapshots::new(
            &[&snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
        assert!(matches!(
            crate::daily_price_only::discovery::discover_session_sources(
                &live, &discovery_request()),
            Err(E::SourceConflict)
        ));
        assert_eq!(snapshot.batches().len(), 4);
    }
}

#[test]
fn later_action_and_calendar_originals_are_not_silently_omitted_by_old_time() {
    for calendar in [false, true] {
        let mut batches = complete_discovery_batches();
        let mut later = if calendar {
            let mut batch = Batch::new(4, DAY);
            batch.add(ResponseKind::Calendar, "calendar.json",
                "quotations/chk-holiday", "CTCA0903R",
                vec![("BASS_DT".into(), "20261012".into()), ("CTX_AREA_FK".into(), "".into()),
                    ("CTX_AREA_NK".into(), "".into())],
                json!({"rt_cd":"0","output":[{"bass_dt":"20261012","opnd_yn":"N"}]}));
            batch
        } else {
            action_batch(4, "2026-10-10", DAY, false)
        };
        later.entry.retrieved_at = time("2026-10-13T08:00:00Z");
        batches.push(later);
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let old_time = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        assert!(matches!(
            crate::daily_price_only::discovery::discover_session_sources(
                &old_time, &discovery_request()),
            Err(E::FutureRetrieval)
        ));
        assert_eq!(snapshot.batches().len(), 4);
    }
}

const ENTITLEMENT: &str = "fixture:licensed";
#[test]
fn identical_historical_manifest_repeat_does_not_omit_or_duplicate_source_rows() {
    let mut batches = complete_discovery_batches();
    batches.push(price_batch(1));
    let fixture = Fixture::new(&batches);
    let before = fixture.fingerprint();
    let snapshot = fixture.scan();
    assert_eq!(snapshot.stats().manifest_records, 4);
    assert_eq!(snapshot.batches().len(), 3);
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let discovered = crate::daily_price_only::discovery::discover_session_sources(
        &raw.evidence, &discovery_request()).unwrap();
    assert_eq!(discovered.session().bars.len(), 11);
    assert_eq!(discovered.session().reference.len(), 11);
    raw.revalidate().unwrap();
    assert_eq!(before, fixture.fingerprint());
}
const DAY: &str = "2026-10-12";

#[test]
fn historical_v3_pin_hashes_the_actual_terminated_line_only() {
    use sha2::{Digest, Sha256};
    let line = b"{\"unicode\":\"\xe9\x9f\x93\",\"n\":1}\n";
    let mut manifest = b"{\"old\":true}\n".to_vec();
    manifest.extend_from_slice(line);
    let expected = ContentHash::parse(&format!("sha256:{:x}", Sha256::digest(line))).unwrap();
    assert_eq!(original_terminated_line_hash(&manifest, 2, &line[..line.len()-1]), Ok(expected));
    assert!(original_terminated_line_hash(&manifest, 0, line).is_err());
    assert!(original_terminated_line_hash(&manifest, 1, &line[..line.len()-1]).is_err());
    assert!(original_terminated_line_hash(&manifest[..manifest.len()-1], 2, &line[..line.len()-1]).is_err());
    assert!(original_terminated_line_hash(&manifest, 2, b"{\"unicode\":\"different\",\"n\":1}").is_err());
    assert_ne!(ContentHash::from_bytes(line), ContentHash::from_bytes(&line[..line.len()-1]));
}

#[test]
fn historical_v2_reader_pin_is_typed_serialization_not_physical_jsonl() {
    use crate::range_to_canonical::HistoricalRawRead;
    let batch = price_batch(1);
    let compact = serde_json::to_vec(&batch.entry).unwrap();
    let pretty = serde_json::to_vec_pretty(&batch.entry).unwrap();
    let fixture = Fixture::new(&[batch]);
    let raw = crate::RawStore::new(&fixture.root);
    let entries = HistoricalRawRead::read_committed_manifest(&raw, "kis", "kr").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(serde_json::to_vec(&entries[0]).unwrap(), compact);
    assert_ne!(ContentHash::from_bytes(&compact), ContentHash::from_bytes(&pretty));
    let files = HistoricalRawRead::read_batch_bytes(&raw, "kis", "kr", &entries[0]).unwrap();
    assert_eq!(files.len(), 22);
    for file in files {
        let original = entries[0].files.iter().find(|entry| entry.file_name == file.file_name).unwrap();
        assert_eq!(ContentHash::from_bytes(&file.bytes), original.content_hash);
    }
}

#[test]
fn historical_reader_interface_rejects_changed_actual_payload() {
    use crate::range_to_canonical::HistoricalRawRead;
    let fixture = Fixture::new(&[price_batch(1)]);
    let raw = crate::RawStore::new(&fixture.root);
    let entries = HistoricalRawRead::read_committed_manifest(&raw, "kis", "kr").unwrap();
    let path = fixture.files.iter().find(|path| path.file_name().unwrap() == "bar-069500.json").unwrap();
    fs::write(path, b"changed-original").unwrap();
    assert!(HistoricalRawRead::read_batch_bytes(&raw, "kis", "kr", &entries[0]).is_err());
}

#[test]
fn historical_equivalent_duplicate_keeps_physical_line_commitments() {
    let first = price_batch(1);
    let mut line = serde_json::to_vec(&first.entry).unwrap();
    line.push(b'\n');
    let fixture = Fixture::new(&[first, price_batch(1)]);
    let snapshot = fixture.scan();
    assert_eq!(snapshot.batches().len(), 1);
    assert_eq!(snapshot.stats().manifest_records, 2);
    let mut expected = line.clone();
    expected.extend_from_slice(&line);
    assert_eq!(snapshot.manifest_bytes(), expected);
    assert_eq!(original_terminated_line_hash(snapshot.manifest_bytes(), 1, &line[..line.len()-1]),
        original_terminated_line_hash(snapshot.manifest_bytes(), 2, &line[..line.len()-1]));
}

#[test]
fn historical_v3_pin_finds_the_actual_equivalent_repeat_without_reserializing_it() {
    let batch = price_batch(1);
    let compact = serde_json::to_vec(&batch.entry).unwrap();
    let mut exact = b"  ".to_vec(); exact.extend_from_slice(&compact); exact.push(b'\n');
    let pin = ContentHash::from_bytes(&exact);
    let mut manifest = compact.clone(); manifest.push(b'\n'); manifest.extend_from_slice(&exact);
    assert_eq!(pinned_terminated_line_hash(&manifest, &batch.entry, &pin), Ok(pin.clone()));
    let mut changed = batch.entry.clone(); changed.entitlement_reference = None;
    assert!(pinned_terminated_line_hash(&manifest, &changed, &pin).is_err());
    manifest.truncate(compact.len()+1);
    assert!(pinned_terminated_line_hash(&manifest, &batch.entry, &pin).is_err());
}

#[test]
fn historical_conflicting_duplicate_is_not_a_second_accepted_original() {
    let clean_fixture = Fixture::new(&[price_batch(1)]);
    let limits = clean_fixture.scan().limits().values();
    let first = price_batch(1);
    let mut changed = price_batch(1);
    changed.entry.entitlement_reference = Some("different-license".to_owned());
    let fixture = Fixture::new(&[first, changed]);
    assert!(CommittedRawReader::new(&fixture.root).snapshot(
        ApprovedRawScope::new("kis", "kr", fixture.uid).unwrap(),
        ScanLimits::new(limits).unwrap()).is_err());
}

fn historical_copy_pair() -> (Batch, Batch) {
    let original = price_batch(1);
    let mut copy = price_batch(2);
    copy.entry.retrieved_at = time("2026-10-12T08:31:00Z");
    let file = &original.entry.files[0];
    copy.entry.files[0].copied_from = Some(SourceFileReference {
        provider: original.entry.provider.clone(), market: original.entry.market.clone(),
        batch_id: original.entry.batch_id, file_name: file.file_name.clone(),
        content_hash: file.content_hash.clone(), retrieved_at: original.entry.retrieved_at,
    });
    (original, copy)
}

#[test]
fn historical_copy_checks_actual_original_bytes_and_every_storage_identity() {
    for mutation in 0..11 {
        let (mut original, mut copy) = historical_copy_pair();
        let name = original.entry.files[0].file_name.clone();
        assert!(verify_historical_copy(&copy.entry, &copy.entry.files[0], &copy.bodies[&name],
            &original.entry, &original.entry.files[0], &original.bodies[&name]).is_ok());
        match mutation {
            0 => original.entry.entitlement_reference = None,
            1 => original.entry.date = date("2026-10-11"),
            2 => original.entry.retrieved_at = time("2026-10-13T08:31:00Z"),
            3 => original.entry.files[0].request.query.push(("extra".into(), "value".into())),
            4 => original.entry.files[0].response_continuation = Some("F".into()),
            5 => original.entry.files[0].kind = ResponseKind::Reference,
            6 => original.entry.files[0].size_bytes += 1,
            7 => { original.bodies.insert(name.clone(), b"changed".to_vec()); },
            8 => original.entry.files[0].copied_from = copy.entry.files[0].copied_from.clone(),
            9 => copy.entry.batch_id = original.entry.batch_id,
            _ => copy.entry.files[0].copied_from.as_mut().unwrap().file_name = "missing.json".into(),
        }
        assert_eq!(verify_historical_copy(&copy.entry, &copy.entry.files[0], &copy.bodies[&name],
            &original.entry, &original.entry.files[0], &original.bodies[&name]), Err(ArchiveSourceError::Pin));
    }
}

#[test]
fn historical_copy_resolves_real_retained_source_and_rejects_dangling_link() {
    let (original, copy) = historical_copy_pair();
    let reference = copy.entry.files[0].copied_from.clone().unwrap();
    let fixture = Fixture::new(&[original, copy]);
    let snapshot = fixture.scan();
    let source = snapshot.lookup_exact(&reference).unwrap();
    assert_eq!(source.content_hash(), &reference.content_hash);
    let mut missing = reference.clone();
    missing.batch_id = BatchId::from_uuid(Uuid::from_u128(999));
    assert!(snapshot.lookup_exact(&missing).is_err());
}

#[test]
fn historical_fixed_providers_do_not_broaden_daily_scope() {
    for provider in ["kis-daily-range", "kis-daily-range-normalized"] {
        assert!(historical_provider(provider).is_ok());
        assert!(ApprovedRawScope::new(provider, "kr", 10001).is_err());
    }
    assert!(historical_provider("arbitrary-provider").is_err());
    assert!(historical_provider("../kis").is_err());
}

fn historical_retained_fixture(attempt: &Attempt) -> (TempDir, RetainedFiles) {
    let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    for name in ["manifest.json", "bars.ndjson"] {
        fs::write(directory.path().join(name), b"actual-retained-original\n").unwrap();
        fs::set_permissions(directory.path().join(name), fs::Permissions::from_mode(0o600)).unwrap();
    }
    let owner = fs::metadata(directory.path()).unwrap().uid();
    let files = RetainedFiles::open_with_attempt(directory.path(), &["manifest.json", "bars.ndjson"],
        owner, 0o600, 1024, Some(attempt)).unwrap();
    (directory, files)
}

#[test]
fn historical_checked_original_read_and_retirement_use_real_descriptors() {
    let attempt = Attempt::default();
    let (_directory, files) = historical_retained_fixture(&attempt);
    assert_eq!(files.read_original(0, 1024).unwrap(), b"actual-retained-original\n");
    files.retire().unwrap();
    assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
}

#[test]
fn historical_callback_replacement_and_manifest_mutation_invalidate_provisional_use() {
    for replacement in [false, true] {
        let attempt = Attempt::default();
        let (directory, files) = historical_retained_fixture(&attempt);
        let path = directory.path().join(if replacement { "bars.ndjson" } else { "manifest.json" });
        let result = historical_callback(&attempt, &attempt, &mut || true, || files.revalidate(), || {
            if replacement { fs::remove_file(&path).unwrap(); }
            fs::write(&path, b"changed-original\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            Ok(())
        });
        assert_eq!(result, Err(ArchiveSourceError::Custody));
        assert!(!attempt.check());
        assert!(files.retire().is_err());
    }
}

#[test]
fn historical_callback_error_cancellation_and_unwind_keep_attempt_failed() {
    for mode in 0..3 {
        let attempt = Attempt::default();
        let (_directory, files) = historical_retained_fixture(&attempt);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            historical_callback(&attempt, &attempt, &mut || mode != 1, || files.revalidate(), || {
                if mode == 2 { panic!("fixture callback unwind"); }
                Err(ArchiveSourceError::Reconstruction)
            })
        }));
        if mode == 2 { assert!(result.is_err()); } else { assert!(result.unwrap().is_err()); }
        assert!(!attempt.check());
        assert!(files.retire().is_err());
    }
}

#[test]
fn historical_retained_reader_rejects_wrong_owner_and_late_custody_gap() {
    let attempt = Attempt::default();
    let (directory, files) = historical_retained_fixture(&attempt);
    let actual = fs::metadata(directory.path()).unwrap().uid();
    assert!(RetainedFiles::open_with_attempt(directory.path(), &["manifest.json", "bars.ndjson"],
        actual.checked_add(1).unwrap(), 0o600, 1024, Some(&attempt)).is_err());
    attempt.gap(crate::checked_close::Gap::Abandoned);
    assert!(files.read_original(0, 1024).is_err());
    assert!(files.retire().is_err());
}

#[test]
fn historical_callback_rejects_different_actual_attempt() {
    let first = Attempt::default();
    let second = Attempt::default();
    let called = std::cell::Cell::new(false);
    assert_eq!(historical_callback(&first, &second, &mut || true, || Ok(()), || {
        called.set(true); Ok(())
    }), Err(ArchiveSourceError::Custody));
    assert!(!called.get());
}

#[test]
fn historical_v3_directory_uses_the_existing_bare_digest_contract() {
    let hash = ContentHash::from_bytes(b"fixed-path-vector");
    assert_eq!(crate::historical_price_only_v3_artifact::candidate_directory_name(&hash).unwrap(),
        hash.as_str().strip_prefix("sha256:").unwrap());
}

#[test]
fn historical_retained_v2_v3_readers_reject_unapproved_actual_originals() {
    let attempt = Attempt::default();
    let (directory, files) = historical_retained_fixture(&attempt);
    let original = HistoricalArtifactView { files: &files, path: directory.path() };
    let v2 = crate::historical_price_only_approval::embedded_v2_archive_pins().unwrap();
    let v3 = crate::historical_price_only_approval::embedded_v3_archive_pins().unwrap();
    assert!(crate::historical_price_only_artifact::read_retained_historical_v2(
        &original, v2.candidate_content_sha256()).is_err());
    assert!(crate::historical_price_only_v3_artifact::read_retained_historical_v3(
        &original, v3.candidate_content_sha256()).is_err());
    assert_eq!(files.read_original(0, 1024).unwrap(), b"actual-retained-original\n");
    files.retire().unwrap();
    assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
}

#[test]
fn historical_v3_real_raw_hashes_cannot_be_replaced_by_a_reviewed_hash_claim() {
    use crate::range_to_canonical::HistoricalRawRead;
    let fixture = Fixture::new(&[price_batch(1)]);
    let raw = crate::RawStore::new(&fixture.root);
    let entries = HistoricalRawRead::read_committed_manifest(&raw, "kis", "kr").unwrap();
    let files = HistoricalRawRead::read_batch_bytes(&raw, "kis", "kr", &entries[0]).unwrap();
    let batch_path = fixture.files.iter().find(|p| p.file_name().unwrap() == "batch.json").unwrap();
    let manifest_path = fixture.files.iter().find(|p| p.file_name().unwrap() == "manifest.jsonl").unwrap();
    let batch_hash = ContentHash::from_bytes(&fs::read(batch_path).unwrap());
    let line_hash = ContentHash::from_bytes(&fs::read(manifest_path).unwrap());
    assert_ne!(batch_hash, ContentHash::parse(crate::range_to_canonical_v3_price::HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_JSON_SHA256).unwrap());
    assert!(crate::range_to_canonical_v3_price::verify_historical_price_only_v3_price_input(
        &entries[0], &files, &batch_hash, &line_hash).is_err());
    assert!(crate::range_to_canonical_v3::verify_historical_price_only_v3_action_input(
        &entries[0], &files, &batch_hash, &line_hash).is_err());
}

#[test]
fn historical_v2_real_reader_reaches_and_rejects_omitted_normalized_originals() {
    use crate::range_to_canonical::{HistoricalRawRead, RangeCanonicalError};
    let mut batch = price_batch(1);
    batch.entry.provider = "kis-daily-range".into();
    batch.entry.batch_id = crate::range_to_canonical::HISTORICAL_PRICE_ONLY_BETA_SOURCE_BATCH_ID.parse().unwrap();
    let fixture = Fixture::new(&[batch]);
    let raw_directory = fixture.root.join("raw");
    for parent in [&raw_directory, &raw_directory.join("manifests")] {
        fs::rename(parent.join("provider=kis"), parent.join("provider=kis-daily-range")).unwrap();
    }
    let normalized = fixture.root.join("raw/manifests/provider=kis-daily-range-normalized/market=kr");
    private_dir(&fixture.root, &normalized);
    for name in ["manifest.jsonl", "commit.lock"] {
        fs::write(normalized.join(name), b"").unwrap();
        fs::set_permissions(normalized.join(name), fs::Permissions::from_mode(0o600)).unwrap();
    }
    let raw = crate::RawStore::new(&fixture.root);
    let entries = HistoricalRawRead::read_committed_manifest(&raw, "kis-daily-range", "kr").unwrap();
    let pin = ContentHash::from_bytes(&serde_json::to_vec(&entries[0]).unwrap());
    let result = crate::range_to_canonical::verify_historical_price_only_beta_input_from_reader(
        &raw, &pin, &ContentHash::from_bytes(b"unreached-action-pin"));
    assert!(matches!(result, Err(RangeCanonicalError::HistoricalBetaContract { reason })
        if reason.starts_with("deterministic normalized session ") && reason.ends_with(" is missing")));
}

#[test]
fn historical_consumer_without_actual_native_binding_never_reads_or_calls_back() {
    let fixture = Fixture::new(&[price_batch(1)]);
    let values = fixture.scan().limits().values();
    for version in [HistoricalArchiveVersion::V2, HistoricalArchiveVersion::V3] {
        let attempt = Attempt::default();
        let invocation = crate::raw_custody::RawInvocation::new(&attempt);
        let called = std::cell::Cell::new(false);
        let root = Path::new("/no-historical-originals-are-created");
        assert!(consume_historical_originals(&invocation, &attempt, &attempt, version,
            root, Path::new("/no-historical-coordination-is-created"), root, root,
            values, || true, |_| { called.set(true); Ok(()) }).is_err());
        assert!(!called.get());
        assert!(!attempt.check());
    }
}

#[test]
fn historical_raw_to_artifact_bar_join_rejects_mutation_deletion_and_duplicate() {
    use domain::{FixedPoint, InstrumentId, Venue};
    let number = |text| FixedPoint::parse(text).unwrap();
    let source = crate::RangeCanonicalBarCandidate {
        instrument_id: InstrumentId::from_parts("069500", Venue::Krx).unwrap(), session_date: date(DAY),
        open: number("100"), high: number("120"), low: number("90"), close: number("110"),
        volume: 200, trading_value: Some(number("22000")),
    };
    // Independently spelled fixture: this proves a join, never an approved dataset.
    let artifact = crate::HistoricalPriceOnlyBar {
        instrument_id: InstrumentId::from_parts("069500", Venue::Krx).unwrap(), session_date: date(DAY),
        raw_open: number("100.0000"), raw_high: number("120.0000"), raw_low: number("90.0000"),
        raw_close: number("110.0000"), raw_volume: 200, raw_trading_value: Some(number("22000.0000")),
        adjusted_open: number("100"), adjusted_high: number("120"), adjusted_low: number("90"), adjusted_close: number("110"),
    };
    assert!(compare_historical_raw_bars(&[source.clone()], &[artifact.clone()]).is_ok());
    let mut changed = artifact.clone(); changed.raw_close = number("111");
    assert!(compare_historical_raw_bars(&[source.clone()], &[changed]).is_err());
    assert!(compare_historical_raw_bars(&[source.clone()], &[]).is_err());
    assert!(compare_historical_raw_bars(&[source.clone(), source], &[artifact.clone(), artifact]).is_err());
}

#[test]
fn historical_callback_late_checked_close_error_is_not_success() {
    struct FailingClose;
    impl crate::checked_close::Close for FailingClose {
        fn close(self) -> Result<(), crate::checked_close::Errno> { Err(crate::checked_close::Errno::Io) }
    }
    let attempt = Attempt::default();
    let (_directory, files) = historical_retained_fixture(&attempt);
    assert_eq!(historical_callback(&attempt, &attempt, &mut || true, || files.revalidate(), || {
        assert!(!attempt.adopt(FailingClose, crate::checked_close::Role::Source).finish());
        Ok(())
    }), Err(ArchiveSourceError::Custody));
    assert!(files.retire().is_err());
    let failure = attempt.finish(Ok::<(), ()>(())).err().unwrap();
    assert_eq!(failure.report().unknown(), 1);
    assert_eq!(failure.report().live(), 0);
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


#[test]
fn calendar_original_reconstructs_exact_source_commitment_and_rows() {
    let mut batch = Batch::new(1, DAY);
    batch.add(ResponseKind::Calendar, "calendar.json", "quotations/chk-holiday", "CTCA0903R",
        vec![("BASS_DT".into(), "20261012".into()), ("CTX_AREA_FK".into(), "".into()),
            ("CTX_AREA_NK".into(), "".into())],
        json!({"rt_cd":"0","output":[{"bass_dt":"20261012","opnd_yn":"Y"}]}));
    let fixture = Fixture::new(&[batch]);
    let snapshot = fixture.scan();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let originals = resolve_kind(&snapshot, ResponseKind::Calendar);
    let original = originals[0].evidence();
    let owner = Uuid::from_u128(9);
    let policy = ContentHash::from_bytes(b"retained-policy");
    let commitment = format!("{{\"schema\":\"owner-beta-daily-normalized-source-v1\",\"kind\":\"calendar\",\"session_date\":\"{DAY}\",\"source_files\":[{{\"provider\":\"kis\",\"market\":\"kr\",\"batch_id\":\"00000000-0000-0000-0000-000000000001\",\"file_name\":\"calendar.json\",\"wire_sha256\":\"{}\"}}]}}", original.wire_sha256);
    let source_hash = ContentHash::from_bytes(commitment.as_bytes());
    let expected_bytes = format!("{{\"kind\":\"calendar\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[{{\"date\":\"{DAY}\",\"evidence_kind\":\"REQUESTED_TARGET\",\"session_type\":\"TRADING\"}}],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"{DAY}\",\"source_evidence_sha256\":\"{source_hash}\"}}");
    let projection = crate::daily_price_only::calendar::project_calendar(&originals[0], owner, &policy).unwrap();
    let document = projection.document(date(DAY)).unwrap();
    assert_eq!(document.descriptor().source_evidence_sha256, source_hash);
    assert_eq!(document.descriptor().sha256, ContentHash::from_bytes(expected_bytes.as_bytes()));
    check_recorded_calendar_document(&raw, owner, &policy, original, document.descriptor())
        .unwrap().consume(|actual| { assert_eq!(actual, expected_bytes.as_bytes()); Ok(()) }).unwrap();
    let mut forged = document.descriptor().clone();
    forged.sha256 = ContentHash::from_bytes(b"jointly-forged-calendar");
    forged.size_bytes = 100;
    assert!(matches!(check_recorded_calendar_document(&raw, owner, &policy, original, &forged),
        Err(ArchiveSourceError::Pin)));
}

#[test]
fn actual_seven_action_originals_reconstruct_empty_rows_with_all_witnesses() {
    let fixture = Fixture::new(&[action_batch(1, "2026-10-10", DAY, false)]);
    let snapshot = fixture.scan();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let reconstructed = crate::daily_price_only::reconstruct::reconstruct_actions(
        &raw.evidence, date("2026-10-09"), date(DAY)).unwrap();
    assert_eq!(reconstructed.witnesses().len(), 7);
    let expected = format!("{{\"kind\":\"corporate_actions\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"{DAY}\",\"source_evidence_sha256\":\"{}\"}}",
        reconstructed.document().descriptor().source_evidence_sha256);
    check_recorded_action_document(&raw, date("2026-10-09"), date(DAY),
        reconstructed.witnesses(), reconstructed.document().descriptor()).unwrap()
        .consume(|actual| { assert_eq!(actual, expected.as_bytes()); Ok(()) }).unwrap();
    let mut witnesses = reconstructed.witnesses().to_vec();
    witnesses.pop();
    assert!(matches!(check_recorded_action_document(&raw, date("2026-10-09"), date(DAY),
        &witnesses, reconstructed.document().descriptor()), Err(ArchiveSourceError::Pin)));
}

#[test]
fn deleted_action_original_cannot_be_consumed() {
    let fixture = Fixture::new(&[action_batch(1, "2026-10-10", DAY, false)]);
    let snapshot = fixture.scan();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let original = crate::daily_price_only::reconstruct::reconstruct_actions(
        &raw.evidence, date("2026-10-09"), date(DAY)).unwrap();
    let checked = check_recorded_action_document(&raw, date("2026-10-09"), date(DAY),
        original.witnesses(), original.document().descriptor()).unwrap();
    let path = fixture.files.iter().find(|p| p.file_name().unwrap() == "corporate-actions-dividend-page-01.json").unwrap();
    assert_eq!(checked.consume(|_| { fs::remove_file(path).unwrap(); Ok(()) }), Err(ArchiveSourceError::Custody));
}


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


fn expected_document(kind: &str, descriptor: &DailyNormalizedDocumentV1Wire) -> Vec<u8> {
    let mut symbols = KR_ETF_CORE_SYMBOLS;
    symbols.sort_unstable();
    let rows = symbols.iter().map(|symbol| match kind {
        "bars" => format!("{{\"close\":110,\"date\":\"{DAY}\",\"high\":120,\"instrument\":\"{symbol}.KRX\",\"low\":90.0,\"open\":100.0,\"value\":22000,\"volume\":200}}"),
        "reference" => format!("{{\"instrument\":\"{symbol}.KRX\",\"provider_symbol\":\"{symbol}\"}}"),
        _ => unreachable!(),
    }).collect::<Vec<_>>().join(",");
    format!("{{\"kind\":\"{kind}\",\"normalizer_version\":\"daily-raw-reconstruction-v1\",\"rows\":[{rows}],\"schema\":\"owner-beta-daily-reconstructed-document-v1\",\"session_date\":\"{DAY}\",\"source_evidence_sha256\":\"{}\"}}", descriptor.source_evidence_sha256).into_bytes()
}

#[test]
fn recorded_prices_use_actual_raw_and_exact_float_document_bytes() {
    let fixture = Fixture::new(&[price_batch(1)]);
    let before = fixture.fingerprint();
    let snapshot = fixture.scan();
    let recorded = prices(&snapshot).unwrap();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let bars = expected_document("bars", recorded.bars().descriptor());
    let reference = expected_document("reference", recorded.reference().descriptor());
    assert_eq!(ContentHash::from_bytes(&bars), recorded.bars().descriptor().sha256);
    assert_eq!(bars.len() as u64, recorded.bars().descriptor().size_bytes);
    check_recorded_price_documents(&raw, date(DAY), recorded.bar_sources(),
        recorded.reference_sources(), recorded.bars().descriptor(), recorded.reference().descriptor())
        .unwrap().consume(|actual_bars, actual_reference| {
            assert_eq!(actual_bars, bars);
            assert_eq!(actual_reference, reference);
            Ok(())
        }).unwrap();
    assert_eq!(fixture.fingerprint(), before);
}

#[test]
fn forged_descriptor_and_changed_original_binding_are_rejected() {
    let fixture = Fixture::new(&[price_batch(1)]);
    let snapshot = fixture.scan();
    let recorded = prices(&snapshot).unwrap();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let mut descriptor = recorded.bars().descriptor().clone();
    descriptor.sha256 = ContentHash::from_bytes(b"forged-normalized-document");
    descriptor.size_bytes = 100;
    assert!(matches!(check_recorded_price_documents(&raw, date(DAY),
        recorded.bar_sources(), recorded.reference_sources(), &descriptor,
        recorded.reference().descriptor()), Err(ArchiveSourceError::Pin)));
    let mut claims = recorded.bar_sources().to_vec();
    claims[0].wire_sha256 = ContentHash::from_bytes(b"other-original");
    assert!(check_recorded_price_documents(&raw, date(DAY), &claims,
        recorded.reference_sources(), recorded.bars().descriptor(),
        recorded.reference().descriptor()).is_err());
    claims = recorded.bar_sources().to_vec();
    claims.pop();
    assert!(matches!(check_recorded_price_documents(&raw, date(DAY), &claims,
        recorded.reference_sources(), recorded.bars().descriptor(),
        recorded.reference().descriptor()), Err(ArchiveSourceError::Reconstruction)));
}

#[test]
fn exact_recorded_sources_survive_later_conflicting_delivery_without_reselection() {
    let original = price_batch(1);
    let mut conflict = price_batch(2);
    conflict.edit_body("bar-069500.json", |value| value["output2"][0]["stck_clpr"] = json!("111"));
    let old_fixture = Fixture::new(&[price_batch(1)]);
    let old_snapshot = old_fixture.scan();
    let recorded = prices(&old_snapshot).unwrap();
    let fixture = Fixture::new(&[original, conflict]);
    let snapshot = fixture.scan();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    check_recorded_price_documents(&raw, date(DAY), recorded.bar_sources(),
        recorded.reference_sources(), recorded.bars().descriptor(),
        recorded.reference().descriptor()).unwrap().consume(|bars, reference| {
            assert_eq!(bars, recorded.bars().bytes());
            assert_eq!(reference, recorded.reference().bytes());
            Ok(())
        }).unwrap();
}

#[test]
fn mutation_during_consuming_use_invalidates_result() {
    let fixture = Fixture::new(&[price_batch(1)]);
    let snapshot = fixture.scan();
    let recorded = prices(&snapshot).unwrap();
    let raw = retain_raw_evidence(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let checked = check_recorded_price_documents(&raw, date(DAY), recorded.bar_sources(),
        recorded.reference_sources(), recorded.bars().descriptor(), recorded.reference().descriptor()).unwrap();
    let source = fixture.files.iter().find(|path| path.file_name().unwrap() == "bar-069500.json").unwrap();
    let outcome = checked.consume(|_, _| {
        fs::write(source, b"changed-original").unwrap();
        Ok(7)
    });
    assert_eq!(outcome, Err(ArchiveSourceError::Custody));
}

#[test]
fn replacement_deletion_and_new_member_invalidate_retained_raw() {
    for mutation in 0..3 {
        let fixture = Fixture::new(&[price_batch(1)]);
        let snapshot = fixture.scan();
        let source = fixture.files.iter().find(|path| path.file_name().unwrap() == "bar-069500.json").unwrap();
        match mutation {
            0 => {
                let bytes = fs::read(source).unwrap();
                fs::remove_file(source).unwrap();
                fs::write(source, bytes).unwrap();
                fs::set_permissions(source, fs::Permissions::from_mode(0o600)).unwrap();
            }
            1 => fs::remove_file(source).unwrap(),
            _ => fs::write(source.parent().unwrap().join("extra.json"), b"{}").unwrap(),
        }
        assert!(snapshot.revalidate().is_err());
    }
}

#[test]
fn changed_raw_lock_and_directory_mode_invalidate_retained_raw() {
    for mutation in 0..2 {
        let fixture = Fixture::new(&[price_batch(1)]);
        let snapshot = fixture.scan();
        let lock = fixture.root.join("raw/manifests/provider=kis/market=kr/commit.lock");
        if mutation == 0 {
            fs::remove_file(&lock).unwrap();
            fs::write(&lock, b"").unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        } else {
            fs::set_permissions(lock.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(snapshot.revalidate().is_err());
    }
}

#[test]
fn wrong_entitlement_and_duplicate_scope_cannot_construct_raw_borrow() {
    let fixture = Fixture::new(&[price_batch(1)]);
    let snapshot = fixture.scan();
    assert!(retain_raw_evidence(&[&snapshot, &snapshot], ENTITLEMENT, observed()).is_err());
    assert!(retain_raw_evidence(&[], ENTITLEMENT, observed()).is_err());
    let raw = retain_raw_evidence(&[&snapshot], "different-rights", observed()).unwrap();
    let recorded = prices(&snapshot).unwrap();
    assert!(check_recorded_price_documents(&raw, date(DAY), recorded.bar_sources(),
        recorded.reference_sources(), recorded.bars().descriptor(), recorded.reference().descriptor()).is_err());
}

#[test]
fn retained_v2_member_custody_rejects_replacement_extra_member_and_mode_change() {
    // This tests retained original custody, not a synthetic V2 approval.
    // Stay beneath the actual package's protected ancestry, never /tmp's 1777.
    for mutation in 0..3 {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["manifest.json", "bars.ndjson"] {
            fs::write(directory.path().join(name), b"exact-test-original\n").unwrap();
            fs::set_permissions(directory.path().join(name), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let owner = fs::metadata(directory.path()).unwrap().uid();
        let retained = RetainedFiles::open(directory.path(), &["manifest.json", "bars.ndjson"],
            owner, 0o600, 1024).unwrap();
        retained.revalidate().unwrap();
        let leaf = directory.path().join("bars.ndjson");
        match mutation {
            0 => {
                fs::remove_file(&leaf).unwrap();
                fs::write(&leaf, b"exact-test-original\n").unwrap();
                fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
            }
            1 => fs::write(directory.path().join("extra"), b"extra").unwrap(),
            _ => fs::set_permissions(&leaf, fs::Permissions::from_mode(0o640)).unwrap(),
        }
        assert!(retained.revalidate().is_err());
    }
}

#[test]
fn retained_original_reader_rejects_path_and_size_bounds_before_returning_custody() {
    assert!(matches!(RetainedFiles::open(Path::new("relative"), &["manifest.json"],
        0, 0o600, 1024), Err(ArchiveSourceError::Context)));
    let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let file = directory.path().join("manifest.json");
    fs::write(&file, b"too-long").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let owner = fs::metadata(directory.path()).unwrap().uid();
    assert!(RetainedFiles::open(directory.path(), &["manifest.json"], owner, 0o600, 1).is_err());
}


// Candidate-view fixtures deliberately use one complete captured Raw tree.
// The claims below are navigation hypotheses, not a fabricated sealed approval.
fn recorded_claims(
    snapshot: &CommittedRawSnapshot,
    ids: &[u128],
) -> Vec<DailyFileEvidenceV1Wire> {
    let full = RawEvidenceSnapshots::new(
        &[snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
    let mut files = Vec::new();
    for batch in snapshot.batches() {
        if !ids.iter().any(|id| batch.batch_id() == BatchId::from_uuid(Uuid::from_u128(*id))) {
            continue;
        }
        for file in batch.files() {
            let reference = SourceFileReference {
                provider: batch.entry().provider.clone(), market: batch.entry().market.clone(),
                batch_id: batch.batch_id(), file_name: file.file_name().into(),
                content_hash: file.content_hash().clone(), retrieved_at: batch.retrieved_at(),
            };
            files.push(resolve_original(&full, &reference).unwrap().evidence().clone());
        }
    }
    crate::daily_price_only::observation::canonical_source_observation(&files)
        .unwrap().files().to_vec()
}

fn later_calendar_batch(id: u128, day: &str, compact: &str) -> Batch {
    let mut batch = Batch::new(id, day);
    batch.entry.retrieved_at = time("2026-10-13T08:00:00Z");
    batch.add(ResponseKind::Calendar, "calendar.json",
        "quotations/chk-holiday", "CTCA0903R",
        vec![("BASS_DT".into(), compact.into()), ("CTX_AREA_FK".into(), "".into()),
            ("CTX_AREA_NK".into(), "".into())],
        json!({"rt_cd":"0","output":[{"bass_dt":compact,"opnd_yn":"Y"}]}));
    batch
}

#[test]
fn recorded_view_reuses_exact_price_calendar_action_readers() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let fixture = Fixture::new(&complete_discovery_batches());
    let before = fixture.fingerprint();
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    let whole = crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()).unwrap();
    let selected = crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    assert_eq!(whole.session(), selected.session());
    assert_eq!(whole.observation().preimage(), selected.observation().preimage());
    assert_eq!(selected.prices().bars().bytes(),
        expected_document("bars", selected.prices().bars().descriptor()));
    assert_eq!(selected.prices().reference().bytes(),
        expected_document("reference", selected.prices().reference().descriptor()));
    assert_eq!(selected.session().calendar.len(), 3);
    assert_eq!(selected.actions().witnesses().len(), 7);
    assert_eq!(selected.documents().len(), 6);
    candidate.view().revalidate_originals().unwrap();
    assert_eq!(before, fixture.fingerprint());
}

#[test]
fn recorded_view_excludes_only_a_navigation_suffix_until_independent_replay() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    for kind in 0..3 {
        let mut batches = complete_discovery_batches();
        let later = match kind {
            0 => {
                let mut value = price_batch(4);
                value.entry.entitlement_reference = Some("other-retained-rights".into());
                value
            }
            1 => action_batch(4, "2026-10-13", "2026-10-13", false),
            _ => later_calendar_batch(4, "2026-10-13", "20261013"),
        };
        batches.push(later);
        let fixture = Fixture::new(&batches);
        let before = fixture.fingerprint();
        let snapshot = fixture.scan();
        let full = RawEvidenceSnapshots::new(
            &[&snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
        let claims = recorded_claims(&snapshot, &[1, 2, 3]);
        let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
        let selected = crate::daily_price_only::discovery::discover_session_sources(
            candidate.view(), &discovery_request()).unwrap();
        let whole = crate::daily_price_only::discovery::discover_session_sources(
            &full, &discovery_request()).unwrap();
        assert_eq!(whole.session(), selected.session());
        assert_eq!(whole.observation().preimage(), selected.observation().preimage());
        assert_eq!(snapshot.batches().len(), 4);
        assert_eq!(snapshot.stats().manifest_records, 4);
        candidate.view().revalidate_originals().unwrap();
        assert_eq!(before, fixture.fingerprint());
    }
}

#[test]
fn recorded_prefix_matching_observation_cannot_prove_later_price_exclusion() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    for conflicting in [false, true] {
        let mut batches = complete_discovery_batches();
        let mut later = price_batch(4);
        later.entry.retrieved_at = time("2026-10-13T08:00:00Z");
        if conflicting {
            later.edit_body("bar-069500.json", |body| body["output2"][0]["stck_clpr"] = json!("111"));
        }
        batches.push(later);
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let full = RawEvidenceSnapshots::new(
            &[&snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
        let claims = recorded_claims(&snapshot, &[1, 2, 3]);
        let committed = crate::daily_price_only::observation::canonical_source_observation(&claims).unwrap();
        let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
        let selected = crate::daily_price_only::discovery::discover_session_sources(
            candidate.view(), &discovery_request()).unwrap();
        assert_eq!(selected.observation().preimage(), committed.preimage());
        assert_eq!(selected.prices().bars().bytes(),
            expected_document("bars", selected.prices().bars().descriptor()));
        // Same retained bytes fit both "A admitted before B" and "A claimed
        // after B". No field in this observation distinguishes those histories.
        // The production gate's complete pass therefore must remain closed.
        assert!(matches!(crate::daily_price_only::discovery::discover_session_sources(
            &full, &discovery_request()), Err(E::SourceConflict)));
        assert_eq!(snapshot.batches()[3].manifest_line_number(), 4);
        assert_eq!(snapshot.batches().len(), 4);
        snapshot.revalidate().unwrap();
    }
}

#[test]
fn recorded_view_does_not_omit_an_earlier_conflicting_manifest_member() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = vec![price_batch(4)];
    batches.extend(complete_discovery_batches());
    let fixture = Fixture::new(&batches);
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let candidate = RecordedPrefixCandidate::derive(
        &full, &recorded_claims(&snapshot, &[1, 2, 3])).unwrap();
    assert_eq!(snapshot.batches()[0].manifest_line_number(), 1);
    assert_eq!(snapshot.batches()[3].manifest_line_number(), 4);
    assert!(matches!(crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()), Err(E::SourceConflict)));
}

#[test]
fn recorded_view_retrieval_order_is_not_a_capture_boundary() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = complete_discovery_batches();
    let mut later_commit = price_batch(4);
    later_commit.entry.retrieved_at = time("2026-10-12T07:30:00Z");
    batches.push(later_commit);
    let fixture = Fixture::new(&batches);
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let candidate = RecordedPrefixCandidate::derive(
        &full, &recorded_claims(&snapshot, &[1, 2, 3])).unwrap();
    assert!(snapshot.batches()[3].retrieved_at() < snapshot.batches()[0].retrieved_at());
    crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    assert!(matches!(crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()), Err(E::SourceConflict)));
}

#[test]
fn recorded_view_later_calendar_or_action_changes_require_full_replay() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    for calendar in [false, true] {
        let mut batches = complete_discovery_batches();
        let later = if calendar {
            later_calendar_batch(4, DAY, "20261012")
        } else {
            action_batch(4, "2026-10-10", DAY, false)
        };
        batches.push(later);
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let full = RawEvidenceSnapshots::new(
            &[&snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
        let claims = recorded_claims(&snapshot, &[1, 2, 3]);
        let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
        let selected = crate::daily_price_only::discovery::discover_session_sources(
            candidate.view(), &discovery_request()).unwrap();
        match crate::daily_price_only::discovery::discover_session_sources(&full, &discovery_request()) {
            Ok(whole) => assert_ne!(whole.observation().preimage(), selected.observation().preimage()),
            Err(_) => {} // A full-reader conflict is also a closed proof branch.
        }
        assert_eq!(snapshot.batches().len(), 4);
    }
}

#[test]
fn recorded_view_exact_manifest_repeats_preserve_original_bytes_and_line_identity() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = complete_discovery_batches();
    batches.push(price_batch(1));
    let fixture = Fixture::new(&batches);
    let before = fixture.fingerprint();
    let snapshot = fixture.scan();
    let manifest = snapshot.manifest_bytes().to_vec();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    assert_eq!(snapshot.stats().manifest_records, 4);
    assert_eq!(snapshot.batches().len(), 3);
    assert_eq!(snapshot.batches()[0].manifest_line_number(), 1);
    let selected = crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    let whole = crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()).unwrap();
    assert_eq!(whole.observation().preimage(), selected.observation().preimage());
    assert_eq!(snapshot.manifest_bytes(), manifest);
    assert_eq!(before, fixture.fingerprint());
}

#[test]
fn recorded_view_includes_forward_copy_original_in_its_actual_scope() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut original = price_batch(4);
    original.entry.retrieved_at = time("2026-10-12T07:30:00Z");
    let mut copied = price_batch(1);
    for file in &mut copied.entry.files {
        let origin = original.entry.files.iter().find(|value| value.file_name == file.file_name).unwrap();
        file.copied_from = Some(SourceFileReference {
            provider: "kis".into(), market: "kr".into(), batch_id: original.entry.batch_id,
            file_name: origin.file_name.clone(), content_hash: origin.content_hash.clone(),
            retrieved_at: original.entry.retrieved_at,
        });
    }
    let mut batches = complete_discovery_batches();
    batches[0] = copied;
    batches.push(original);
    let fixture = Fixture::new(&batches);
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    assert!(claims.iter().any(|claim| !claim.copied_from.is_empty()));
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    let source = &batches[3].entry;
    let file = &source.files[0];
    let reference = SourceFileReference {
        provider: source.provider.clone(), market: source.market.clone(),
        batch_id: source.batch_id, file_name: file.file_name.clone(),
        content_hash: file.content_hash.clone(), retrieved_at: source.retrieved_at,
    };
    assert!(resolve_original(candidate.view(), &reference).is_ok());
    assert!(matches!(crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()), Err(E::SourceConflict)));
    assert_eq!(snapshot.batches()[3].manifest_line_number(), 4);
}

#[test]
fn recorded_view_rejects_missing_original_even_when_a_later_copy_matches() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = complete_discovery_batches();
    batches.push(price_batch(4));
    let fixture = Fixture::new(&batches);
    let snapshot = fixture.scan();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let old = fixture.files.iter().find(|path| {
        path.file_name().unwrap() == "bar-069500.json"
            && path.parent().unwrap().file_name().unwrap().to_string_lossy()
                == format!("batch={}", batches[0].entry.batch_id)
    }).unwrap();
    fs::remove_file(old).unwrap();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    assert!(RecordedPrefixCandidate::derive(&full, &claims).is_err());
}

#[test]
fn recorded_view_rejects_substituted_claims_wrong_rights_and_nested_selection() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let fixture = Fixture::new(&complete_discovery_batches());
    let snapshot = fixture.scan();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    for change in 0..4 {
        let mut changed = claims.clone();
        match change {
            0 => changed[0].manifest_line_sha256 = ContentHash::from_bytes(b"other-line"),
            1 => changed[0].wire_sha256 = ContentHash::from_bytes(b"other-original"),
            2 => changed[0].batch_id = BatchId::from_uuid(Uuid::from_u128(999)),
            _ => changed.clear(),
        }
        assert!(RecordedPrefixCandidate::derive(&full, &changed).is_err());
    }
    let wrong = RawEvidenceSnapshots::new(&[&snapshot], "wrong-rights", observed()).unwrap();
    assert!(RecordedPrefixCandidate::derive(&wrong, &claims).is_err());
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    assert!(RecordedPrefixCandidate::derive(candidate.view(), &claims).is_err());
}

#[test]
fn recorded_view_excluded_original_and_manifest_mutation_invalidates_full_custody() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    for change in 0..3 {
        let mut batches = complete_discovery_batches();
        let mut extra = price_batch(4);
        extra.entry.entitlement_reference = Some("other-retained-rights".into());
        batches.push(extra);
        let fixture = Fixture::new(&batches);
        let snapshot = fixture.scan();
        let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
        let candidate = RecordedPrefixCandidate::derive(
            &full, &recorded_claims(&snapshot, &[1, 2, 3])).unwrap();
        let target = if change == 0 {
            fixture.files.iter().rev().find(|path| path.file_name().unwrap() == "bar-069500.json").unwrap().clone()
        } else if change == 1 {
            fixture.root.join("raw/manifests/provider=kis/market=kr/manifest.jsonl")
        } else {
            fixture.root.join("raw/manifests/provider=kis/market=kr/commit.lock")
        };
        if change == 2 {
            fs::remove_file(&target).unwrap();
        }
        fs::write(&target, b"changed-retained-original").unwrap();
        assert!(candidate.view().revalidate_originals().is_err());
        assert!(full.revalidate_originals().is_err());
    }
}


#[test]
fn recorded_view_rejects_duplicate_and_noncanonical_claim_order() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let fixture = Fixture::new(&complete_discovery_batches());
    let snapshot = fixture.scan();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let mut duplicate = claims.clone();
    duplicate.insert(1, duplicate[0].clone());
    assert!(matches!(RecordedPrefixCandidate::derive(&full, &duplicate), Err(E::InvalidEvidence)));
    let mut reversed = claims;
    reversed.reverse();
    assert!(matches!(RecordedPrefixCandidate::derive(&full, &reversed), Err(E::InvalidEvidence)));
}

#[test]
fn recorded_view_storage_equivalent_duplicate_line_is_not_rewritten() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = complete_discovery_batches();
    batches.push(price_batch(1));
    let fixture = Fixture::new(&batches);
    let path = fixture.root.join("raw/manifests/provider=kis/market=kr/manifest.jsonl");
    let bytes = fs::read(&path).unwrap();
    let mut lines = bytes.split(|byte| *byte == b'\n').filter(|line| !line.is_empty())
        .map(|line| line.to_vec()).collect::<Vec<_>>();
    lines[3].insert(0, b' ');
    lines[3].push(b' ');
    let mut original = Vec::new();
    for line in &lines {
        original.extend_from_slice(line);
        original.push(b'\n');
    }
    fs::write(&path, &original).unwrap();
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
    let candidate = RecordedPrefixCandidate::derive(
        &full, &recorded_claims(&snapshot, &[1, 2, 3])).unwrap();
    assert_eq!(snapshot.stats().manifest_records, 4);
    assert_eq!(snapshot.batches().len(), 3);
    assert_eq!(snapshot.manifest_bytes(), original);
    let selected = crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    let whole = crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()).unwrap();
    assert_eq!(whole.observation().preimage(), selected.observation().preimage());
    candidate.view().revalidate_originals().unwrap();
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn recorded_view_scope_boundaries_are_derived_independently_from_actual_originals() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut main = complete_discovery_batches();
    let mut calendar = main.pop().unwrap();
    calendar.entry.provider = "kis-calendar".into();
    let mut unrelated = price_batch(4);
    unrelated.entry.entitlement_reference = Some("other-retained-rights".into());
    main.push(unrelated);
    let mut later = later_calendar_batch(5, "2026-10-13", "20261013");
    later.entry.provider = "kis-calendar".into();
    let main_fixture = Fixture::new(&main);
    let calendar_fixture = Fixture::new(&[calendar, later]);
    // Fixture construction is complete before either actual Raw capture.
    // No captured owner or committed original is renamed to create a prior view.
    fs::rename(calendar_fixture.root.join("raw/provider=kis"),
        calendar_fixture.root.join("raw/provider=kis-calendar")).unwrap();
    fs::rename(calendar_fixture.root.join("raw/manifests/provider=kis"),
        calendar_fixture.root.join("raw/manifests/provider=kis-calendar")).unwrap();
    let main_snapshot = main_fixture.scan();
    let calendar_snapshot = calendar_fixture.scan_provider("kis-calendar");
    let mut claims = recorded_claims(&main_snapshot, &[1, 2]);
    claims.extend(recorded_claims(&calendar_snapshot, &[3]));
    let claims = crate::daily_price_only::observation::canonical_source_observation(&claims)
        .unwrap().files().to_vec();
    let full = RawEvidenceSnapshots::new(&[&calendar_snapshot, &main_snapshot],
        ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    let selected = crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    let whole = crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()).unwrap();
    assert_eq!(selected.session(), whole.session());
    assert_eq!(selected.observation().preimage(), whole.observation().preimage());
    assert_eq!(main_snapshot.batches().len(), 3);
    assert_eq!(calendar_snapshot.batches().len(), 2);
    assert_eq!(main_snapshot.batches()[1].manifest_line_number(), 2);
    assert_eq!(calendar_snapshot.batches()[0].manifest_line_number(), 1);
    candidate.view().revalidate_originals().unwrap();
    let missing_scope = RawEvidenceSnapshots::new(&[&main_snapshot], ENTITLEMENT, observed()).unwrap();
    assert!(RecordedPrefixCandidate::derive(&missing_scope, &claims).is_err());
    let calendar_path = calendar_fixture.root.join(format!(
        "raw/provider=kis-calendar/market=kr/date=2026-10-13/batch={}/calendar.json",
        BatchId::from_uuid(Uuid::from_u128(5))));
    fs::write(calendar_path, b"changed-excluded-other-scope").unwrap();
    assert!(candidate.view().revalidate_originals().is_err());
}


#[test]
fn recorded_prefix_one_later_file_is_a_concrete_exclusion_counterexample() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    let mut batches = complete_discovery_batches();
    let mut later = price_batch(4);
    later.entry.files.retain(|file| file.file_name == "bar-069500.json");
    later.bodies.retain(|name, _| name == "bar-069500.json");
    later.entry.retrieved_at = time("2026-10-13T08:00:00Z");
    later.edit_body("bar-069500.json", |body| body["output2"][0]["stck_clpr"] = json!("111"));
    batches.push(later);
    let fixture = Fixture::new(&batches);
    let snapshot = fixture.scan();
    let full = RawEvidenceSnapshots::new(
        &[&snapshot], ENTITLEMENT, time("2026-10-14T08:00:00Z")).unwrap();
    let claims = recorded_claims(&snapshot, &[1, 2, 3]);
    let observation = crate::daily_price_only::observation::canonical_source_observation(&claims).unwrap();
    let candidate = RecordedPrefixCandidate::derive(&full, &claims).unwrap();
    let selected = crate::daily_price_only::discovery::discover_session_sources(
        candidate.view(), &discovery_request()).unwrap();
    assert_eq!(snapshot.batches()[3].files().count(), 1);
    assert_eq!(snapshot.batches()[3].manifest_line_number(), 4);
    assert_eq!(selected.observation().preimage(), observation.preimage());
    assert_eq!(ContentHash::from_bytes(selected.prices().bars().bytes()),
        selected.prices().bars().descriptor().sha256);
    assert!(matches!(crate::daily_price_only::discovery::discover_session_sources(
        &full, &discovery_request()), Err(E::SourceConflict)));
    // The actual log/observation bytes cannot say whether the recorded claim
    // was made before or after line 4 became visible. A complete-source replay
    // rejects this ambiguity; prefix equality alone would incorrectly accept.
    full.revalidate_originals().unwrap();
}

#[test]
fn recorded_view_callback_success_cancellation_and_mutation_check_all_originals() {
    use crate::daily_price_only::verify::RecordedPrefixCandidate;
    for change in [false, true] {
        for cancelled in [false, true] {
            let mut batches = complete_discovery_batches();
            let mut excluded = price_batch(4);
            excluded.entry.entitlement_reference = Some("other-retained-rights".into());
            batches.push(excluded);
            let fixture = Fixture::new(&batches);
            let snapshot = fixture.scan();
            let full = RawEvidenceSnapshots::new(&[&snapshot], ENTITLEMENT, observed()).unwrap();
            let candidate = RecordedPrefixCandidate::derive(
                &full, &recorded_claims(&snapshot, &[1, 2, 3])).unwrap();
            let path = fixture.files.iter().rev()
                .find(|path| path.file_name().unwrap() == "bar-069500.json").unwrap();
            let called = std::cell::Cell::new(false);
            let outcome = candidate.view().with_original_custody(
                |_| "ORIGINAL_CHANGED",
                |sources| {
                    called.set(true);
                    let selected = crate::daily_price_only::discovery::discover_session_sources(
                        sources, &discovery_request()).unwrap();
                    assert_eq!(selected.prices().bars().bytes(),
                        expected_document("bars", selected.prices().bars().descriptor()));
                    if change {
                        fs::write(path, b"changed-excluded-original-in-callback").unwrap();
                    }
                    if cancelled { Err("CANCELLED") } else { Ok(()) }
                },
            );
            assert!(called.get());
            assert_eq!(outcome, if change { Err("ORIGINAL_CHANGED") }
                else if cancelled { Err("CANCELLED") } else { Ok(()) });
            if change {
                called.set(false);
                assert_eq!(candidate.view().with_original_custody(
                    |_| "ORIGINAL_CHANGED",
                    |_| { called.set(true); Ok(()) },
                ), Err("ORIGINAL_CHANGED"));
                assert!(!called.get());
            }
        }
    }
}


// SOURCE_ONLY / NOT_RUN: these exercise real retained files through the newly
// borrowed interface; they do not manufacture a native binding or V3 approval.
#[test]
fn captured_publication_interface_reads_exact_originals_and_duplicate_membership() {
    use crate::storage::RawOriginalRead;
    let mut batches = complete_discovery_batches(); batches.push(price_batch(1));
    let fixture = Fixture::new(&batches); let before = fixture.fingerprint();
    let original = fixture.scan(); let scopes = [&original];
    let reader = CapturedPublicationReader { originals: &scopes, keep_reading: &|| true };
    let entries = reader.read_committed_manifest("kis", "kr").unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(original.stats().manifest_records, 4);
    let entry = entries.iter().find(|entry| entry.batch_id == batches[0].entry.batch_id).unwrap();
    let files = reader.read_batch_bytes("kis", "kr", entry).unwrap();
    assert_eq!(files.len(), batches[0].entry.files.len());
    for file in files { assert_eq!(file.bytes, batches[0].bodies[&file.file_name]); }
    assert_eq!(before, fixture.fingerprint());
    original.revalidate().unwrap();
}
#[test]
fn captured_publication_interface_refuses_detached_metadata_duplicate_scope_and_unknown_provider() {
    use crate::storage::RawOriginalRead;
    let fixture = Fixture::new(&[price_batch(1)]); let original = fixture.scan();
    let scopes = [&original]; let reader = CapturedPublicationReader { originals: &scopes, keep_reading: &|| true };
    let mut entry = reader.read_committed_manifest("kis", "kr").unwrap().remove(0);
    entry.files[0].content_hash = ContentHash::from_bytes(b"same-shaped-caller-hash");
    assert!(reader.read_batch_bytes("kis", "kr", &entry).is_err());
    assert!(reader.read_committed_manifest("caller-provider", "kr").is_err());
    assert!(reader.read_committed_manifest("kis", "us").is_err());
    let duplicates = [&original, &original];
    let reader = CapturedPublicationReader { originals: &duplicates, keep_reading: &|| true };
    assert!(reader.read_committed_manifest("kis", "kr").is_err());
}
#[test]
fn captured_publication_reader_invalidates_provisional_rows_after_same_byte_replacement() {
    use crate::storage::RawOriginalRead;
    let fixture = Fixture::new(&[price_batch(1)]); let original = fixture.scan(); let scopes = [&original];
    let reader = CapturedPublicationReader { originals: &scopes, keep_reading: &|| true };
    let entries = reader.read_committed_manifest("kis", "kr").unwrap();
    let first = reader.read_batch_bytes("kis", "kr", &entries[0]).unwrap();
    let path = &first[0].storage_path; let bytes = fs::read(path).unwrap();
    let old = path.with_extension("retained-old"); fs::rename(path, &old).unwrap();
    fs::write(path, bytes).unwrap(); fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(reader.revalidate().is_err());
    assert!(reader.read_batch_bytes("kis", "kr", &entries[0]).is_err());
}
#[test]
fn captured_publication_reader_cancellation_checks_before_returning_original_bytes() {
    use crate::storage::RawOriginalRead;
    let fixture = Fixture::new(&[price_batch(1)]); let original = fixture.scan(); let scopes = [&original];
    let called = std::cell::Cell::new(0);
    let keep = || { let old = called.get(); called.set(old + 1); old == 0 };
    let reader = CapturedPublicationReader { originals: &scopes, keep_reading: &keep };
    assert!(reader.read_committed_manifest("kis", "kr").is_err());
    assert!(called.get() >= 2);
}
#[test]
fn daily_archive_scope_requires_real_native_binding_before_any_open_or_callback() {
    for foreign in [false, true] {
        let actual = Attempt::default(); let other = Attempt::default();
        let invocation = crate::RawInvocation::new(&actual); let called = std::cell::Cell::new(false);
        let result = crate::daily_price_only::artifact::with_archive_daily_roots(&invocation, &actual,
            if foreign { &other } else { &actual }, Path::new("/must-not-open-admissions"), Path::new("/must-not-open-frontiers"),
            |_, _| { called.set(true); Ok(()) });
        assert!(result.is_err()); assert!(!called.get());
        let failure = actual.finish(Ok::<(), ()>(())).err().unwrap();
        assert_eq!(failure.report().attempted(), 0); assert_eq!(failure.report().live(), 0);
        if foreign { assert!(!other.check()); }
    }
}
#[test]
fn checked_raw_borrow_empty_is_not_an_empty_original_authority() {
    let actual = Attempt::default(); let other = Attempt::default(); let called = std::cell::Cell::new(false);
    let result = crate::daily_price_only::source::with_checked_raw_originals(&[], &actual, &other, |_| {
        called.set(true); Ok::<(), ()>(())
    });
    let failure = result.err().expect("empty original membership refuses");
    assert!(!called.get()); assert!(failure.callback.is_none());
    assert!(failure.before.is_empty()); assert!(failure.after.is_empty());
    assert!(!actual.check()); assert!(!other.check());
}

#[test]
fn ordinary_original_holders_retain_send_sync_api_traits() {
    fn require_send_sync<T: Send + Sync>() {}
    require_send_sync::<CommittedRawSnapshot>();
    require_send_sync::<crate::daily_price_only::artifact::DailyAdmissionRoot>();
    require_send_sync::<crate::daily_price_only::artifact::DailySourceFrontierRoot>();
}
