//! Operational source identities, without frontier or admission authority.
//! Encoding claims grants neither source existence nor protected-seal authority.

use chrono::Datelike;
use domain::{ContentHash, TradingDate};
use uuid::Uuid;

use super::DailyFileEvidenceV1Wire;
use super::contract::{canonical_file_evidence, fixed_limits};
use super::verify::DailyVerificationError;

const OBSERVATION_PREFIX: &[u8] = b"owner-beta-daily-source-observation-v1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailySourceKind {
    Action,
    Calendar,
    Bar,
    Reference,
}

impl DailySourceKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Action => "ACTION",
            Self::Calendar => "CALENDAR",
            Self::Bar => "BAR",
            Self::Reference => "REFERENCE",
        }
    }
}

/// Vocabulary only: SOURCE_REVOKED still requires the separate guarded operator
/// revocation path; block_owner_beta_daily_input does not accept that code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyBlockCode {
    SourceConflict,
    SealedSourceChanged,
    SourceRevoked,
    SourceIntegrity,
    CalendarConflict,
    NoncashAction,
}

impl DailyBlockCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::SourceConflict => "SOURCE_CONFLICT",
            Self::SealedSourceChanged => "SEALED_SOURCE_CHANGED",
            Self::SourceRevoked => "SOURCE_REVOKED",
            Self::SourceIntegrity => "SOURCE_INTEGRITY",
            Self::CalendarConflict => "CALENDAR_CONFLICT",
            Self::NoncashAction => "NONCASH_ACTION",
        }
    }
}

/// Validated encoding of claims, not proof that a source exists. Callers must
/// attribute claims to captured sources or protected expected evidence before
/// recording an event. An unattributable parse/IO failure is only a poll failure.
pub struct CanonicalSourceObservation {
    files: Vec<DailyFileEvidenceV1Wire>,
    preimage: Vec<u8>,
    sha256: ContentHash,
}

impl CanonicalSourceObservation {
    pub fn files(&self) -> &[DailyFileEvidenceV1Wire] {
        &self.files
    }
    pub fn preimage(&self) -> &[u8] {
        &self.preimage
    }
    pub fn sha256(&self) -> &ContentHash {
        &self.sha256
    }
}

/// D5 A6: full canonical D4-byte ordering; only byte-identical repeats disappear.
/// Conflicting variants of a single file identity remain in the observation.
pub fn canonical_source_observation(
    files: &[DailyFileEvidenceV1Wire],
) -> Result<CanonicalSourceObservation, DailyVerificationError> {
    canonical_source_observation_with_limit(files, fixed_limits().max_manifest_bytes)
}

fn canonical_source_observation_with_limit(
    files: &[DailyFileEvidenceV1Wire],
    max_bytes: u64,
) -> Result<CanonicalSourceObservation, DailyVerificationError> {
    let limits = fixed_limits();
    if files.is_empty() {
        return Err(DailyVerificationError::InvalidEvidence);
    }
    if files.len() as u64 > limits.max_file_evidence_per_session {
        return Err(DailyVerificationError::LimitExceeded);
    }
    let mut total = OBSERVATION_PREFIX.len() as u64 + 2;
    let mut encoded = Vec::new();
    for file in files {
        let (canonical, bytes) =
            canonical_file_evidence(file).map_err(|_| DailyVerificationError::InvalidEvidence)?;
        total = total
            .checked_add(bytes.len() as u64)
            .and_then(|value| value.checked_add(u64::from(!encoded.is_empty())))
            .ok_or(DailyVerificationError::LimitExceeded)?;
        if total > max_bytes {
            return Err(DailyVerificationError::LimitExceeded);
        }
        encoded.push((bytes, canonical));
    }
    encoded.sort_by(|left, right| left.0.cmp(&right.0));
    encoded.dedup_by(|left, right| left.0 == right.0);
    let capacity = usize::try_from(total).map_err(|_| DailyVerificationError::LimitExceeded)?;
    let mut preimage = Vec::new();
    preimage
        .try_reserve(capacity)
        .map_err(|_| DailyVerificationError::LimitExceeded)?;
    preimage.extend_from_slice(OBSERVATION_PREFIX);
    preimage.push(b'[');
    let mut canonical_files = Vec::new();
    for (bytes, file) in encoded {
        if !canonical_files.is_empty() {
            preimage.push(b',');
        }
        preimage.extend_from_slice(&bytes);
        canonical_files.push(file);
    }
    preimage.push(b']');
    let sha256 = ContentHash::from_bytes(&preimage);
    Ok(CanonicalSourceObservation {
        files: canonical_files,
        preimage,
        sha256,
    })
}

pub struct CanonicalSourceEvent {
    kind: DailySourceKind,
    code: DailyBlockCode,
    range_start: TradingDate,
    range_end: TradingDate,
    observation_sha256: ContentHash,
    preimage: Vec<u8>,
    sha256: ContentHash,
}

impl CanonicalSourceEvent {
    pub fn kind(&self) -> DailySourceKind {
        self.kind
    }
    pub fn code(&self) -> DailyBlockCode {
        self.code
    }
    pub fn range_start(&self) -> TradingDate {
        self.range_start
    }
    pub fn range_end(&self) -> TradingDate {
        self.range_end
    }
    pub fn observation_sha256(&self) -> &ContentHash {
        &self.observation_sha256
    }
    pub fn preimage(&self) -> &[u8] {
        &self.preimage
    }
    pub fn sha256(&self) -> &ContentHash {
        &self.sha256
    }
}

/// Exact SQL0060 event preimage, with final LF. No time, release, candidate ID,
/// random salt, provider text or error serialization participates in its identity.
pub fn canonical_source_event(
    owner: Uuid,
    kind: DailySourceKind,
    code: DailyBlockCode,
    range_start: TradingDate,
    range_end: TradingDate,
    observation: &CanonicalSourceObservation,
) -> Result<CanonicalSourceEvent, DailyVerificationError> {
    if owner.is_nil()
        || range_end < range_start
        || !(1..=9999).contains(&range_start.as_naive_date().year())
        || !(1..=9999).contains(&range_end.as_naive_date().year())
    {
        return Err(DailyVerificationError::InvalidContext);
    }
    let preimage = format!(
        "owner-beta-daily-source-event-v1\n{owner}\n{}\n{}\n{range_start}\n{range_end}\n{}\n",
        kind.code(),
        code.code(),
        observation.sha256()
    )
    .into_bytes();
    Ok(CanonicalSourceEvent {
        kind,
        code,
        range_start,
        range_end,
        observation_sha256: observation.sha256().clone(),
        sha256: ContentHash::from_bytes(&preimage),
        preimage,
    })
}

/// Pure comparison of canonical evidence claims, not a trusted seal loader.
/// The storage/admitter must supply protected expected evidence and independently
/// discovered current evidence for this exact Owner/range. Empty current evidence
/// is a disappearance, never a successful empty frontier. An unchanged release or
/// observation time is absent from this function's identity entirely.
pub fn compare_source_observation(
    owner: Uuid,
    range_start: TradingDate,
    range_end: TradingDate,
    expected: &CanonicalSourceObservation,
    observed: &[DailyFileEvidenceV1Wire],
) -> Result<Option<CanonicalSourceEvent>, DailyVerificationError> {
    // Validate context even on the replay path.
    if owner.is_nil()
        || range_end < range_start
        || !(1..=9999).contains(&range_start.as_naive_date().year())
        || !(1..=9999).contains(&range_end.as_naive_date().year())
    {
        return Err(DailyVerificationError::InvalidContext);
    }
    let current = if observed.is_empty() {
        None
    } else {
        Some(canonical_source_observation(observed)?)
    };
    if current
        .as_ref()
        .is_some_and(|value| value.preimage() == expected.preimage())
    {
        return Ok(None);
    }
    let mut variants = std::collections::BTreeMap::new();
    for (old, files) in [
        (true, expected.files()),
        (
            false,
            current.as_ref().map_or(&[][..], |value| value.files()),
        ),
    ] {
        for file in files {
            let (_, bytes) = canonical_file_evidence(file)
                .map_err(|_| DailyVerificationError::InvalidEvidence)?;
            let entry = variants.entry(bytes).or_insert((file, false, false));
            if old {
                entry.1 = true;
            } else {
                entry.2 = true;
            }
        }
    }
    let changed = variants
        .values()
        .find(|(_, old, new)| old != new)
        .ok_or(DailyVerificationError::InvalidEvidence)?
        .0;
    let kind = match changed.kind {
        super::DailyEvidenceKindV1::Bars => DailySourceKind::Bar,
        super::DailyEvidenceKindV1::Reference => DailySourceKind::Reference,
        super::DailyEvidenceKindV1::Calendar => DailySourceKind::Calendar,
        super::DailyEvidenceKindV1::CorporateActions => DailySourceKind::Action,
    };
    // Union retains both conflicting variants and missing expected commitments;
    // no corrupt provider body or fabricated source identity is needed.
    let files: Vec<_> = variants
        .into_values()
        .map(|(file, _, _)| file.clone())
        .collect();
    let observation = canonical_source_observation(&files)?;
    canonical_source_event(
        owner,
        kind,
        DailyBlockCode::SealedSourceChanged,
        range_start,
        range_end,
        &observation,
    )
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::super::{
        DailyEvidenceKindV1, DailyFileOriginV1Wire, DailyRequestMetadataV1Wire,
        DailyWireTimestampV1,
    };
    use super::*;
    use domain::BatchId;

    const GOLDEN_FILE: &str = r#"{"provider":"kis","market":"kr","batch_id":"00000000-0000-4000-8000-000000000001","batch_date":"2026-10-12","batch_json_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000001","manifest_line_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000002","file_name":"calendar.json","kind":"calendar","wire_sha256":"sha256:0000000000000000000000000000000000000000000000000000000000000003","size_bytes":17,"request":{"endpoint":"/uapi/domestic-stock/v1/quotations/chk-holiday","query":[["BASS_DT","20261012"],["CTX_AREA_FK",""],["CTX_AREA_NK",""]],"headers":[["appkey","[REDACTED]"],["appsecret","[REDACTED]"],["authorization","[REDACTED]"],["tr_cont",""],["tr_id","CTCA0903R"]],"mode":"credentialed"},"response_continuation":"M","retrieved_at":"2026-10-12T07:31:00.000000000Z","retrieval_precision":"batch","original_retrieved_at":"2026-10-12T07:31:00.000000000Z","original_retrieval_precision":"batch","copied_from":[]}"#;
    const GOLDEN_OBSERVATION: &str =
        "sha256:4b8e4195b3b84233b98859c8cc210b01ffd77c75a1998c219bc3e5d828c232d7";
    const GOLDEN_EVENT: &str =
        "sha256:dc508451316f0f68515e6d3365e6dfb6f8195ab68803cba5f3f62264eccf9c64";

    fn date(s: &str) -> TradingDate {
        TradingDate::parse(s).unwrap()
    }
    fn owner() -> Uuid {
        Uuid::parse_str("00000000-0000-4000-8000-000000000099").unwrap()
    }
    fn hash(n: u64) -> ContentHash {
        ContentHash::parse(&format!("sha256:{n:064x}")).unwrap()
    }
    fn file() -> DailyFileEvidenceV1Wire {
        let time = DailyWireTimestampV1::parse("2026-10-12T07:31:00.000000000Z").unwrap();
        DailyFileEvidenceV1Wire {
            provider: "kis".into(),
            market: "kr".into(),
            batch_id: BatchId::from_uuid(
                Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            ),
            batch_date: date("2026-10-12"),
            batch_json_sha256: hash(1),
            manifest_line_sha256: hash(2),
            file_name: "calendar.json".into(),
            kind: DailyEvidenceKindV1::Calendar,
            wire_sha256: hash(3),
            size_bytes: 17,
            request: DailyRequestMetadataV1Wire {
                endpoint: "/uapi/domestic-stock/v1/quotations/chk-holiday".into(),
                query: vec![
                    ("BASS_DT".into(), "20261012".into()),
                    ("CTX_AREA_FK".into(), "".into()),
                    ("CTX_AREA_NK".into(), "".into()),
                ],
                headers: vec![
                    ("appkey".into(), "[REDACTED]".into()),
                    ("appsecret".into(), "[REDACTED]".into()),
                    ("authorization".into(), "[REDACTED]".into()),
                    ("tr_cont".into(), "".into()),
                    ("tr_id".into(), "CTCA0903R".into()),
                ],
                mode: "credentialed".into(),
            },
            response_continuation: Some("M".into()),
            retrieved_at: time,
            retrieval_precision: "batch".into(),
            original_retrieved_at: time,
            original_retrieval_precision: "batch".into(),
            copied_from: vec![],
        }
    }
    fn observation(files: &[DailyFileEvidenceV1Wire]) -> CanonicalSourceObservation {
        canonical_source_observation(files).unwrap()
    }
    fn compare(
        expected: &CanonicalSourceObservation,
        current: &[DailyFileEvidenceV1Wire],
    ) -> Option<CanonicalSourceEvent> {
        compare_source_observation(
            owner(),
            date("2026-10-10"),
            date("2026-10-12"),
            expected,
            current,
        )
        .unwrap()
    }

    #[test]
    fn s5_observation_literal_d4_bytes_and_hash_are_pinned() {
        let actual = observation(&[file()]);
        assert_eq!(
            actual.preimage(),
            format!("owner-beta-daily-source-observation-v1\n[{GOLDEN_FILE}]").as_bytes()
        );
        assert_eq!(actual.sha256().as_str(), GOLDEN_OBSERVATION);
        assert_eq!(actual.files(), &[file()]);
        assert!(!actual.preimage().ends_with(b"\n"));
    }
    #[test]
    fn s5_observation_exact_byte_boundary_rejects_one_byte_less() {
        let exact = canonical_source_observation_with_limit(&[file()], 1005).unwrap();
        assert_eq!(exact.preimage().len(), 1005);
        assert_eq!(
            canonical_source_observation_with_limit(&[file()], 1004).err(),
            Some(DailyVerificationError::LimitExceeded)
        );
    }
    #[test]
    fn s5_observation_budget_is_preflighted_before_duplicate_elimination() {
        assert_eq!(
            canonical_source_observation_with_limit(&[file(), file()], 1005).err(),
            Some(DailyVerificationError::LimitExceeded)
        );
        assert_eq!(
            observation(&[file(), file()]).preimage(),
            observation(&[file()]).preimage()
        );
    }

    #[test]
    fn s5_observation_order_pairs_and_identical_repeats_converge() {
        let first = file();
        let mut second = file();
        second.file_name = "second.json".into();
        let expected = observation(&[first.clone(), second.clone()]);
        let mut reordered = first.clone();
        reordered.request.query.reverse();
        reordered.request.headers.reverse();
        let actual = observation(&[second, reordered, first]);
        assert_eq!(expected.preimage(), actual.preimage());
        assert_eq!(actual.files().len(), 2);
    }
    #[test]
    fn s5_observation_conflicting_same_identity_variants_are_retained() {
        let first = file();
        let mut conflict = first.clone();
        conflict.wire_sha256 = hash(4);
        let actual = observation(&[first.clone(), conflict.clone()]);
        assert_eq!(actual.files().len(), 2);
        assert_ne!(actual.sha256(), observation(&[first]).sha256());
        assert_eq!(
            actual.preimage(),
            observation(&[conflict, file()]).preimage()
        );
    }
    #[test]
    fn s5_observation_commits_metadata_and_time_as_well_as_body() {
        let baseline = observation(&[file()]);
        for i in 0..6 {
            let mut changed = file();
            match i {
                0 => changed.batch_json_sha256 = hash(8),
                1 => changed.manifest_line_sha256 = hash(8),
                2 => changed.size_bytes += 1,
                3 => changed.response_continuation = Some("F".into()),
                4 => changed.batch_id = BatchId::from_uuid(Uuid::from_u128(8)),
                _ => {
                    changed.retrieved_at =
                        DailyWireTimestampV1::parse("2026-10-12T07:32:00.000000000Z").unwrap();
                    changed.original_retrieved_at = changed.retrieved_at;
                }
            }
            assert_ne!(baseline.sha256(), observation(&[changed]).sha256());
        }
    }
    #[test]
    fn s5_observation_copy_keeps_original_and_delivery_identities() {
        let original = file();
        let mut copy = original.clone();
        copy.provider = "kis-calendar".into();
        copy.batch_id = BatchId::from_uuid(Uuid::from_u128(9));
        copy.batch_date = date("2026-10-13");
        copy.retrieved_at = DailyWireTimestampV1::parse("2026-10-13T07:31:00.000000000Z").unwrap();
        copy.copied_from.push(DailyFileOriginV1Wire {
            provider: original.provider.clone(),
            market: original.market.clone(),
            batch_id: original.batch_id,
            batch_date: original.batch_date,
            batch_json_sha256: original.batch_json_sha256.clone(),
            manifest_line_sha256: original.manifest_line_sha256.clone(),
            file_name: original.file_name.clone(),
            kind: original.kind,
            wire_sha256: original.wire_sha256.clone(),
            size_bytes: original.size_bytes,
            request: original.request.clone(),
            response_continuation: original.response_continuation.clone(),
            retrieved_at: original.retrieved_at,
            retrieval_precision: original.retrieval_precision.clone(),
        });
        let actual = observation(std::slice::from_ref(&copy));
        assert_ne!(actual.sha256(), observation(&[original]).sha256());
        assert_eq!(actual.files()[0].copied_from.len(), 1);
        copy.copied_from[0].manifest_line_sha256 = hash(12);
        assert_ne!(actual.sha256(), observation(&[copy]).sha256());
    }
    #[test]
    fn s5_observation_empty_or_invalid_wire_claims_are_rejected() {
        assert_eq!(
            canonical_source_observation(&[]).err(),
            Some(DailyVerificationError::InvalidEvidence)
        );
        for i in 0..4 {
            let mut changed = file();
            match i {
                0 => changed.size_bytes = 0,
                1 => changed.file_name = "../secret".into(),
                2 => changed.request.headers[0].1 = "synthetic-unredacted-sentinel".into(),
                _ => changed
                    .request
                    .headers
                    .push(changed.request.headers[0].clone()),
            }
            assert_eq!(
                canonical_source_observation(&[changed]).err(),
                Some(DailyVerificationError::InvalidEvidence)
            );
        }
    }
    #[test]
    fn s5_event_literal_prefix_context_and_final_lf_are_pinned() {
        let obs = observation(&[file()]);
        let actual = canonical_source_event(
            owner(),
            DailySourceKind::Calendar,
            DailyBlockCode::SealedSourceChanged,
            date("2026-10-10"),
            date("2026-10-12"),
            &obs,
        )
        .unwrap();
        assert_eq!(actual.preimage(),format!("owner-beta-daily-source-event-v1\n00000000-0000-4000-8000-000000000099\nCALENDAR\nSEALED_SOURCE_CHANGED\n2026-10-10\n2026-10-12\n{GOLDEN_OBSERVATION}\n").as_bytes());
        assert_eq!(actual.sha256().as_str(), GOLDEN_EVENT);
        assert_eq!(actual.observation_sha256(), obs.sha256());
        assert_eq!(actual.kind(), DailySourceKind::Calendar);
        assert_eq!(actual.code(), DailyBlockCode::SealedSourceChanged);
    }
    #[test]
    fn s5_event_codes_kinds_and_ranges_do_not_alias() {
        let obs = observation(&[file()]);
        let mut hashes = std::collections::BTreeSet::new();
        for kind in [
            DailySourceKind::Action,
            DailySourceKind::Calendar,
            DailySourceKind::Bar,
            DailySourceKind::Reference,
        ] {
            for code in [
                DailyBlockCode::SourceConflict,
                DailyBlockCode::SealedSourceChanged,
                DailyBlockCode::SourceRevoked,
                DailyBlockCode::SourceIntegrity,
                DailyBlockCode::CalendarConflict,
                DailyBlockCode::NoncashAction,
            ] {
                let event = canonical_source_event(
                    owner(),
                    kind,
                    code,
                    date("2026-10-10"),
                    date("2026-10-12"),
                    &obs,
                )
                .unwrap();
                assert!(hashes.insert(event.sha256().as_str().to_owned()));
            }
        }
        assert_eq!(hashes.len(), 24);
        let changed = canonical_source_event(
            owner(),
            DailySourceKind::Calendar,
            DailyBlockCode::SealedSourceChanged,
            date("2026-10-11"),
            date("2026-10-12"),
            &obs,
        )
        .unwrap();
        assert_ne!(changed.sha256().as_str(), GOLDEN_EVENT);
        assert_eq!(changed.range_start(), date("2026-10-11"));
        assert_eq!(changed.range_end(), date("2026-10-12"));
    }
    #[test]
    fn s5_event_invalid_owner_and_reversed_range_fail_closed() {
        let obs = observation(&[file()]);
        for (id, start, end) in [
            (Uuid::nil(), "2026-10-10", "2026-10-12"),
            (owner(), "2026-10-12", "2026-10-10"),
        ] {
            assert_eq!(
                canonical_source_event(
                    id,
                    DailySourceKind::Calendar,
                    DailyBlockCode::SourceIntegrity,
                    date(start),
                    date(end),
                    &obs
                )
                .err(),
                Some(DailyVerificationError::InvalidContext)
            );
            assert_eq!(
                compare_source_observation(id, date(start), date(end), &obs, &[file()]).err(),
                Some(DailyVerificationError::InvalidContext)
            );
        }
    }
    #[test]
    fn s5_identical_frontier_replays_after_reordering_and_duplicate_claims() {
        let mut reordered = file();
        reordered.request.query.reverse();
        assert!(compare(&observation(&[file()]), &[reordered, file()]).is_none());
    }
    #[test]
    fn s5_equal_wire_alternative_identity_is_a_same_date_change() {
        let first = file();
        let mut alternative = first.clone();
        alternative.batch_id = BatchId::from_uuid(Uuid::from_u128(6));
        let event = compare(
            &observation(std::slice::from_ref(&first)),
            &[first, alternative],
        )
        .unwrap();
        assert_eq!(event.code(), DailyBlockCode::SealedSourceChanged);
        assert_eq!(event.kind(), DailySourceKind::Calendar);
    }
    #[test]
    fn s5_complete_source_disappearance_binds_expected_evidence() {
        let expected = observation(&[file()]);
        let event = compare(&expected, &[]).unwrap();
        assert_eq!(event.observation_sha256(), expected.sha256());
        assert_eq!(event.sha256().as_str(), GOLDEN_EVENT);
    }
    #[test]
    fn s5_partial_disappearance_and_mutation_keep_expected_and_observed_union() {
        let first = file();
        let mut second = file();
        second.file_name = "second.json".into();
        let expected = observation(&[first.clone(), second]);
        let missing = compare(&expected, std::slice::from_ref(&first)).unwrap();
        assert_eq!(missing.observation_sha256(), expected.sha256());
        let mut changed = first.clone();
        changed.wire_sha256 = hash(77);
        let changed_expected = observation(std::slice::from_ref(&first));
        let event = compare(&changed_expected, std::slice::from_ref(&changed)).unwrap();
        assert_eq!(
            event.observation_sha256(),
            observation(&[first, changed]).sha256()
        );
    }
    #[test]
    fn s5_change_event_is_independent_of_current_inventory_order() {
        let first = file();
        let mut second = file();
        second.file_name = "second.json".into();
        let mut third = file();
        third.file_name = "third.json".into();
        let expected = observation(&[first]);
        let one = compare(&expected, &[second.clone(), third.clone()]).unwrap();
        let two = compare(&expected, &[third, second]).unwrap();
        assert_eq!(one.preimage(), two.preimage());
    }
}
