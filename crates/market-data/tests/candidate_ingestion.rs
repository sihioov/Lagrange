use domain::{TradingDate, UtcTimestamp};
use market_data::{
    CANDIDATE_RESPONSE_KINDS, CandidateDocument, EodProvider, FetchRequest, IngestError,
    IngestRequest, KrxProvider, RawStore, RecordedBundle, ResponseKind, ingest_bundle_with_kinds,
    parse_candidate_envelope,
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn date() -> TradingDate {
    TradingDate::parse("2026-08-14").expect("valid fixture date")
}

fn retrieved_at() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339("2026-08-14T07:00:00Z").expect("valid fixture timestamp")
}

#[test]
fn candidate_bundle_is_exact_typed_and_immutable() {
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(fixture("kr-candidates/contract")).expect("candidate fixture opens"),
    );
    let fetch = FetchRequest::new("kr".to_owned(), date(), retrieved_at())
        .with_kinds(CANDIDATE_RESPONSE_KINDS);
    let envelopes = provider.fetch(&fetch).expect("candidate fixture fetches");
    assert_eq!(envelopes.len(), CANDIDATE_RESPONSE_KINDS.len());
    let documents = envelopes
        .iter()
        .map(parse_candidate_envelope)
        .collect::<Result<Vec<_>, _>>()
        .expect("every response is a strict typed candidate document");
    assert!(matches!(documents[0], CandidateDocument::InvestorFlow(_)));
    assert!(matches!(documents[1], CandidateDocument::MarketStatus(_)));
    assert!(matches!(documents[2], CandidateDocument::Fundamentals(_)));
    assert!(matches!(
        documents[3],
        CandidateDocument::IndexMembership(_)
    ));
    assert!(matches!(
        documents[4],
        CandidateDocument::SectorClassification(_)
    ));

    let directory = tempfile::tempdir().expect("temporary Raw store");
    let store = RawStore::new(directory.path());
    let request = IngestRequest::new("kr".to_owned(), date(), retrieved_at());
    let outcome = ingest_bundle_with_kinds(
        &store,
        &provider,
        &request,
        Some("synthetic-candidate-contract"),
        &CANDIDATE_RESPONSE_KINDS,
    )
    .expect("candidate delivery is stored");
    assert_eq!(outcome.files.len(), CANDIDATE_RESPONSE_KINDS.len());
    assert_eq!(outcome.entry.files.len(), CANDIDATE_RESPONSE_KINDS.len());
    assert_eq!(
        outcome.entry.entitlement_reference.as_deref(),
        Some("synthetic-candidate-contract")
    );
    assert_eq!(
        outcome
            .entry
            .files
            .iter()
            .map(|file| file.kind)
            .collect::<Vec<_>>(),
        CANDIDATE_RESPONSE_KINDS
    );
}

#[test]
fn explicit_ingestion_fails_closed_on_missing_or_duplicate_capabilities() {
    let provider = KrxProvider::synthetic(
        RecordedBundle::open(fixture("kr-etf/contract")).expect("legacy fixture opens"),
    );
    let directory = tempfile::tempdir().expect("temporary Raw store");
    let store = RawStore::new(directory.path());
    let request = IngestRequest::new("kr".to_owned(), date(), retrieved_at());

    let missing =
        ingest_bundle_with_kinds(&store, &provider, &request, None, &CANDIDATE_RESPONSE_KINDS)
            .expect_err("an empty candidate response set must fail");
    assert!(matches!(missing, IngestError::ResponseShape { .. }));

    let duplicate = ingest_bundle_with_kinds(
        &store,
        &provider,
        &request,
        None,
        &[ResponseKind::InvestorFlow, ResponseKind::InvestorFlow],
    )
    .expect_err("duplicate requested classes must fail before storage");
    assert!(matches!(duplicate, IngestError::ResponseShape { .. }));
    assert!(
        store
            .read_manifest("krx", "kr")
            .expect("manifest remains readable")
            .is_empty()
    );
}
