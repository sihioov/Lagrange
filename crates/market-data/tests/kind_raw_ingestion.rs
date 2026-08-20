//! Raw-only ingestion tests for the KIND ETF disclosure-search **capture**
//! adapter (`market_data::providers::kind`).
//!
//! KIND has no API and no reader trait: this module only ingests bytes a
//! separate browser-capture stage already retrieved. There is therefore no
//! network I/O anywhere in this file — every [`CapturedPage`] here is a
//! small inline HTML fixture, never a real KIND response.

use domain::{ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KIND_DISCLOSURE, ResponseKind};
use market_data::providers::kind::KindCaptureTermination;
use market_data::storage::RawStore;
use market_data::{
    CapturedPage, KIND_DISCLOSURE_MAX_PAGES, KIND_ETF_DISCLOSURE_ENDPOINT, KindError, KindSurface,
    ManifestEntry, ingest_disclosure_capture,
};

const NOW: &str = "2026-08-19T08:00:00Z";

/// Synthetic entitlement reference used by every ingest call in this file.
/// Not a real vault path — only a fixed, obviously-synthetic value
/// satisfying the required, non-empty `entitlement_reference` parameter.
const SYNTHETIC_ENTITLEMENT_REFERENCE: &str = "vault://synthetic-entitlements/kind-test-only.pdf";

fn new_store() -> (tempfile::TempDir, RawStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = RawStore::new(temp.path());
    (temp, store)
}

fn fixed_date() -> TradingDate {
    TradingDate::new(2026, 8, 19).expect("valid date")
}

fn fixed_retrieved_at() -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(NOW).expect("valid timestamp")
}

/// A small inline HTML fragment containing the documented `시간` column
/// header and a couple of rows, distinguished by `marker` so different pages
/// have different bytes. Never a fixtures-directory file — built inline per
/// the task's constraint.
fn fixture_page_html(marker: &str) -> Vec<u8> {
    format!(
        "<table class=\"list\">\
           <tr><th>회사명</th><th>제목</th><th>시간</th></tr>\
           <tr><td>SYNTHETIC ETF ISSUER {marker}-A</td><td>SYNTHETIC DISCLOSURE</td><td>2026-08-19 09:00</td></tr>\
           <tr><td>SYNTHETIC ETF ISSUER {marker}-B</td><td>SYNTHETIC DISCLOSURE</td><td>2026-08-19 09:05</td></tr>\
         </table>"
    )
    .into_bytes()
}

/// The documented `fnSearch()` form fields, as the page itself would send
/// them for a given `pageIndex`.
fn form_fields(page_index: u32) -> Vec<(String, String)> {
    vec![
        (
            "method".to_owned(),
            "searchDisclosureByStockTypeEtfSub".to_owned(),
        ),
        (
            "forward".to_owned(),
            "disclosurebystocktype_etf_sub".to_owned(),
        ),
        ("currentPageSize".to_owned(), "15".to_owned()),
        ("pageIndex".to_owned(), page_index.to_string()),
        ("orderMode".to_owned(), "1".to_owned()),
        ("orderStat".to_owned(), "D".to_owned()),
    ]
}

/// One well-formed captured page for `page_index`, with bytes unique to
/// that index (so a happy-path multi-page capture never accidentally trips
/// the duplicate-bytes guard).
fn captured_page(page_index: u32) -> CapturedPage {
    CapturedPage {
        page_index,
        bytes: fixture_page_html(&page_index.to_string()),
        retrieved_at: fixed_retrieved_at(),
        form_fields: form_fields(page_index),
    }
}

fn manifest_text(store: &RawStore) -> String {
    std::fs::read_to_string(store.manifest_path(PROVIDER_KIND_DISCLOSURE, MARKET_KR))
        .unwrap_or_default()
}

fn batch_dir_file_names(
    store: &RawStore,
    date: &TradingDate,
    entry: &ManifestEntry,
) -> Vec<String> {
    let dir = store.batch_dir(PROVIDER_KIND_DISCLOSURE, MARKET_KR, date, &entry.batch_id);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("batch dir exists")
        .map(|file| {
            file.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn batch_dir_file_bytes(
    store: &RawStore,
    date: &TradingDate,
    entry: &ManifestEntry,
    file_name: &str,
) -> Vec<u8> {
    let dir = store.batch_dir(PROVIDER_KIND_DISCLOSURE, MARKET_KR, date, &entry.batch_id);
    std::fs::read(dir.join(file_name)).expect("read stored file")
}

/// Asserts nothing at all was written for this provider/market: no batch
/// directory tree, no manifest.
fn assert_nothing_written(store: &RawStore) {
    assert!(
        !store
            .provider_dir(PROVIDER_KIND_DISCLOSURE, MARKET_KR)
            .exists(),
        "a rejected capture must leave no provider directory behind"
    );
    assert!(
        !store
            .manifest_path(PROVIDER_KIND_DISCLOSURE, MARKET_KR)
            .exists(),
        "a rejected capture must leave no manifest behind"
    );
}

// ---------------------------------------------------------------------
// Test 1: happy path — three pages, one batch, three files, manifest row,
// and the recorded endpoint/mode/form fields match what was supplied.
// ---------------------------------------------------------------------

#[test]
fn happy_path_three_pages_one_batch_three_files_one_manifest_row() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let pages = vec![captured_page(1), captured_page(2), captured_page(3)];

    let entry = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect("three well-formed pages should ingest cleanly");

    assert_eq!(entry.provider, PROVIDER_KIND_DISCLOSURE);
    assert_eq!(entry.market, MARKET_KR);
    assert_eq!(
        entry.entitlement_reference.as_deref(),
        Some(SYNTHETIC_ENTITLEMENT_REFERENCE)
    );
    assert_eq!(entry.files.len(), 3);

    let names = batch_dir_file_names(&store, &date, &entry);
    assert_eq!(
        names,
        vec![
            "batch.json".to_owned(),
            "page-0001.html".to_owned(),
            "page-0002.html".to_owned(),
            "page-0003.html".to_owned(),
        ]
    );

    for (idx, file) in entry.files.iter().enumerate() {
        let page_index = idx as u32 + 1;
        assert_eq!(file.kind, ResponseKind::DisclosureIndex);
        assert_eq!(file.file_name, format!("page-{page_index:04}.html"));
        assert_eq!(file.request.endpoint, KIND_ETF_DISCLOSURE_ENDPOINT);
        assert_eq!(file.request.mode, FetchMode::Synthetic);
        assert!(file.request.headers.is_empty());
        assert_eq!(file.request.query, form_fields(page_index));
    }

    // The manifest gained exactly one row for this batch.
    let manifest = manifest_text(&store);
    assert_eq!(manifest.lines().count(), 1);
    assert!(manifest.contains(&entry.batch_id.to_string()));
}

// ---------------------------------------------------------------------
// Test 2: stored bytes are byte-for-byte identical to what was supplied,
// and each file's recorded content hash equals a SHA-256 computed
// independently in this test.
// ---------------------------------------------------------------------

#[test]
fn stored_bytes_are_exact_and_content_hash_matches_independent_sha256() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let pages = vec![captured_page(1), captured_page(2)];

    let entry = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect("two well-formed pages should ingest cleanly");

    assert_eq!(pages.len(), entry.files.len());
    for (page, file) in pages.iter().zip(entry.files.iter()) {
        let stored = batch_dir_file_bytes(&store, &date, &entry, &file.file_name);
        assert_eq!(
            stored, page.bytes,
            "stored bytes must equal supplied bytes exactly"
        );

        let independent_hash = ContentHash::from_bytes(&page.bytes);
        assert_eq!(
            file.content_hash, independent_hash,
            "recorded content hash must equal a hash computed independently over the same bytes"
        );
    }
}

// ---------------------------------------------------------------------
// Test 3: empty or whitespace-only entitlement reference fails closed.
// ---------------------------------------------------------------------

#[test]
fn empty_or_whitespace_only_entitlement_reference_fails_closed() {
    for blank in ["", "   ", "\t\n"] {
        let (_temp, store) = new_store();
        let date = fixed_date();
        let pages = vec![captured_page(1)];

        let error = ingest_disclosure_capture(
            &store,
            MARKET_KR,
            &date,
            blank,
            FetchMode::Synthetic,
            KindSurface::EtfList,
            KindCaptureTermination::ClampedDuplicate,
            &pages,
        )
        .expect_err("a blank entitlement reference must fail closed");

        assert!(
            matches!(error, KindError::MissingEntitlementReference),
            "blank={blank:?}: expected MissingEntitlementReference, got {error:?}"
        );
        assert_nothing_written(&store);
    }
}

// ---------------------------------------------------------------------
// Test 4: empty `pages` fails closed.
// ---------------------------------------------------------------------

#[test]
fn empty_pages_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &[],
    )
    .expect_err("an empty capture must fail closed");

    assert!(matches!(error, KindError::EmptyCapture));
    assert_nothing_written(&store);
}

#[test]
fn incomplete_capture_termination_fails_closed_before_raw_storage() {
    for termination in [
        KindCaptureTermination::PageBoundReached,
        KindCaptureTermination::AdvanceControlMissing,
        KindCaptureTermination::NoResponse,
    ] {
        let (_temp, store) = new_store();
        let date = fixed_date();

        let error = ingest_disclosure_capture(
            &store,
            MARKET_KR,
            &date,
            SYNTHETIC_ENTITLEMENT_REFERENCE,
            FetchMode::Synthetic,
            KindSurface::EtfList,
            termination,
            &[captured_page(1)],
        )
        .expect_err("an incomplete capture termination must fail closed");

        assert!(matches!(
            error,
            KindError::IncompleteCapture { termination: actual } if actual == termination
        ));
        assert_nothing_written(&store);
    }
}

#[test]
fn clean_termination_allows_exactly_the_configured_page_bound() {
    let (_temp, store) = new_store();
    let date = fixed_date();
    let pages: Vec<CapturedPage> = (1..=KIND_DISCLOSURE_MAX_PAGES as u32)
        .map(captured_page)
        .collect();

    let entry = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect("a duplicate probe after page 40 proves a 40-page capture complete");

    assert_eq!(entry.files.len(), KIND_DISCLOSURE_MAX_PAGES);
}

// ---------------------------------------------------------------------
// Test 5: page indices that start at 2, that skip a number, or that repeat
// one each fail closed.
// ---------------------------------------------------------------------

#[test]
fn malformed_page_index_sequences_fail_closed() {
    let cases: Vec<(&str, Vec<u32>, u32, u32)> = vec![
        ("starts_at_2", vec![2, 3], 1, 2),
        ("skips_a_number", vec![1, 3], 2, 3),
        ("repeats_one", vec![1, 1, 2], 2, 1),
    ];

    for (label, indices, expected, actual) in cases {
        let (_temp, store) = new_store();
        let date = fixed_date();
        let pages: Vec<CapturedPage> = indices
            .iter()
            .map(|&page_index| CapturedPage {
                page_index,
                bytes: fixture_page_html(&page_index.to_string()),
                retrieved_at: fixed_retrieved_at(),
                form_fields: form_fields(page_index),
            })
            .collect();

        let error = ingest_disclosure_capture(
            &store,
            MARKET_KR,
            &date,
            SYNTHETIC_ENTITLEMENT_REFERENCE,
            FetchMode::Synthetic,
            KindSurface::EtfList,
            KindCaptureTermination::ClampedDuplicate,
            &pages,
        )
        .expect_err(&format!(
            "{label}: malformed page-index sequence must fail closed"
        ));

        match error {
            KindError::PageIndexOutOfSequence {
                expected: got_expected,
                actual: got_actual,
            } => {
                assert_eq!(got_expected, expected, "{label}: unexpected `expected`");
                assert_eq!(got_actual, actual, "{label}: unexpected `actual`");
            }
            other => panic!("{label}: expected PageIndexOutOfSequence, got {other:?}"),
        }
        assert_nothing_written(&store);
    }
}

// ---------------------------------------------------------------------
// Test 6: exceeding KIND_DISCLOSURE_MAX_PAGES fails closed.
// ---------------------------------------------------------------------

#[test]
fn exceeding_max_pages_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let pages: Vec<CapturedPage> = (1..=(KIND_DISCLOSURE_MAX_PAGES as u32 + 1))
        .map(captured_page)
        .collect();

    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect_err("exceeding the page bound must fail closed");

    assert!(matches!(
        error,
        KindError::TooManyPages {
            max_pages: KIND_DISCLOSURE_MAX_PAGES,
            actual
        } if actual == KIND_DISCLOSURE_MAX_PAGES + 1
    ));
    assert_nothing_written(&store);
}

// ---------------------------------------------------------------------
// Test 7: a page whose body lacks the `시간` label fails closed.
// ---------------------------------------------------------------------

#[test]
fn page_missing_time_column_label_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let pages = vec![CapturedPage {
        page_index: 1,
        bytes: b"<table><tr><td>no timestamp column here</td></tr></table>".to_vec(),
        retrieved_at: fixed_retrieved_at(),
        form_fields: form_fields(1),
    }];

    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect_err("a body missing the documented 시간 column must fail closed");

    assert!(matches!(
        error,
        KindError::MissingTimeColumn { page_index: 1 }
    ));
    assert_nothing_written(&store);
}

// ---------------------------------------------------------------------
// Test 8: two pages with identical bytes fail closed.
// ---------------------------------------------------------------------

#[test]
fn identical_bytes_across_two_pages_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let shared_bytes = fixture_page_html("SAME-BYTES");
    let pages = vec![
        CapturedPage {
            page_index: 1,
            bytes: shared_bytes.clone(),
            retrieved_at: fixed_retrieved_at(),
            form_fields: form_fields(1),
        },
        CapturedPage {
            page_index: 2,
            bytes: shared_bytes,
            retrieved_at: fixed_retrieved_at(),
            form_fields: form_fields(2),
        },
    ];

    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect_err("byte-identical pages must fail closed");

    assert!(matches!(
        error,
        KindError::DuplicatePageBytes {
            page_index: 2,
            duplicate_of: 1
        }
    ));
    assert_nothing_written(&store);
}

// ---------------------------------------------------------------------
// Test 9: empty form_fields and credential-like form field names fail closed.
// ---------------------------------------------------------------------

#[test]
fn empty_form_fields_fails_closed() {
    let (_temp, store) = new_store();
    let date = fixed_date();

    let pages = vec![CapturedPage {
        page_index: 1,
        bytes: fixture_page_html("1"),
        retrieved_at: fixed_retrieved_at(),
        form_fields: vec![],
    }];

    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &pages,
    )
    .expect_err("empty form_fields must fail closed");

    assert!(matches!(
        error,
        KindError::EmptyFormFields { page_index: 1 }
    ));
    assert_nothing_written(&store);
}

#[test]
fn credential_like_form_field_name_fails_closed() {
    const SENTINEL_CREDENTIAL_VALUE: &str = "SENTINEL-SECRET-MUST-NEVER-PERSIST-93217";

    let credential_like_names = [
        "api_key",
        "authToken",
        "client_secret",
        "passwd",
        "password",
    ];

    for field_name in credential_like_names {
        let (_temp, store) = new_store();
        let date = fixed_date();

        let mut fields = form_fields(1);
        fields.push((field_name.to_owned(), SENTINEL_CREDENTIAL_VALUE.to_owned()));

        let pages = vec![CapturedPage {
            page_index: 1,
            bytes: fixture_page_html("1"),
            retrieved_at: fixed_retrieved_at(),
            form_fields: fields,
        }];

        let error = ingest_disclosure_capture(
            &store,
            MARKET_KR,
            &date,
            SYNTHETIC_ENTITLEMENT_REFERENCE,
            FetchMode::Synthetic,
            KindSurface::EtfList,
            KindCaptureTermination::ClampedDuplicate,
            &pages,
        )
        .expect_err(&format!(
            "field name {field_name:?} looks credential-shaped and must fail closed"
        ));

        assert!(matches!(
            error,
            KindError::CredentialLikeFormField {
                page_index: 1,
                field_name: recorded_name,
            } if recorded_name == field_name
        ));
        assert_nothing_written(&store);
    }
}

// ---------------------------------------------------------------------
// Test 10: on any rejection, nothing is written — the provider directory
// and manifest do not exist. (Also asserted inline in every rejection test
// above; this test re-confirms it explicitly for one representative
// failure of each broad category.)
// ---------------------------------------------------------------------

#[test]
fn any_rejection_leaves_no_provider_directory_or_manifest_behind() {
    let scenarios: Vec<(&str, Vec<CapturedPage>, &str)> = vec![
        ("empty_pages", vec![], SYNTHETIC_ENTITLEMENT_REFERENCE),
        (
            "bad_sequence",
            vec![captured_page(2)],
            SYNTHETIC_ENTITLEMENT_REFERENCE,
        ),
    ];

    for (label, pages, entitlement_reference) in scenarios {
        let (_temp, store) = new_store();
        let date = fixed_date();

        let result = ingest_disclosure_capture(
            &store,
            MARKET_KR,
            &date,
            entitlement_reference,
            FetchMode::Synthetic,
            KindSurface::EtfList,
            KindCaptureTermination::ClampedDuplicate,
            &pages,
        );
        assert!(result.is_err(), "{label}: expected a fail-closed rejection");
        assert_nothing_written(&store);
    }

    // Blank entitlement reference, checked separately since it needs at
    // least one otherwise-valid page to prove rejection happens for the
    // entitlement check specifically, not for some other reason.
    let (_temp, store) = new_store();
    let date = fixed_date();
    let result = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        "   ",
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &[captured_page(1)],
    );
    assert!(result.is_err());
    assert_nothing_written(&store);
}

// ---------------------------------------------------------------------
// Test 11: `DetailEtf` remains a compatibility surface but is deferred/not
// allowed for this approved Raw provider path.
// ---------------------------------------------------------------------

#[test]
fn detail_etf_surface_is_rejected_before_raw_storage() {
    let date = fixed_date();

    let (_temp, store) = new_store();
    let error = ingest_disclosure_capture(
        &store,
        MARKET_KR,
        &date,
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::DetailEtf,
        KindCaptureTermination::ClampedDuplicate,
        &[captured_page(1)],
    )
    .expect_err("DetailEtf must be rejected by the Raw provider");

    assert!(matches!(
        error,
        KindError::UnsupportedRawSurface {
            surface: KindSurface::DetailEtf
        }
    ));
    assert_nothing_written(&store);
}
