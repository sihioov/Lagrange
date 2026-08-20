//! Offline tests for the KIND disclosure **parsing** normalizer
//! (`market_data::kind_normalize`): stored `provider=kind-disclosure` HTML
//! pages -> typed `provider=kind-disclosure-normalized` observations.
//!
//! Every fixture here is a small inline HTML fragment built in this file —
//! never a fixtures-directory file, and never real KIND bytes. No network
//! I/O happens anywhere in this file.
//!
//! # Fixtures mirror the stored artifact, not a rendered page
//!
//! A real stored KIND disclosure page carries **no `<th>` cells at all**:
//! its `<thead>` holds exactly one empty `<tr>`, and the column labels a
//! rendered page shows are injected into it client-side by
//! `fn_InitTitle(...)`, never present in the bytes this crate reads. The
//! bytes instead carry the same five labels on the `<table>` element's own
//! `summary` attribute. Every fixture below reproduces that shape verbatim
//! (down to the real, inconsistently-spaced `summary` value), plus the two
//! `onclick`-embedded identifiers every real row carries — see
//! `crates/market-data/src/kind_normalize.rs` module docs for the full
//! contract this file is testing against.

use domain::{TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KIND_DISCLOSURE_NORMALIZED};
use market_data::providers::kind::KindCaptureTermination;
use market_data::storage::RawStore;
use market_data::{
    CapturedPage, InstrumentIdentity, KindNormalizeError, KindSurface, ManifestEntry,
    RequiredField, TimezoneAssumption, deterministic_kind_disclosure_normalized_batch_id,
    ingest_disclosure_capture, normalize_kind_disclosure_batch,
};

/// Synthetic entitlement reference used by every ingest call in this file.
/// Not a real vault path — only a fixed, obviously-synthetic value
/// satisfying the required, non-empty `entitlement_reference` parameter.
const SYNTHETIC_ENTITLEMENT_REFERENCE: &str =
    "vault://synthetic-entitlements/kind-normalize-test-only.pdf";
const NOW: &str = "2026-08-19T08:00:00Z";

/// The real, verbatim `summary` attribute value observed on a stored KIND
/// ETF disclosure-search result page — spacing around the commas is
/// inconsistent (`", "` after the first three, `","` before the last) in
/// the real bytes, and that inconsistency is preserved here on purpose: the
/// contract trims each part, so this must still validate.
const KIND_SUMMARY: &str = "번호, 시간, 종목명, 공시제목,제출인";

/// A real disclosure acceptance number sample (`openDisclsViewer`'s first
/// argument), reused whenever a test does not care about its exact value.
const SYNTHETIC_ACCEPTANCE_NUMBER: &str = "20200207000058";
/// A real KIND-internal issue key sample (`etfisusummary_open`'s argument),
/// reused whenever a test does not care about its exact value.
const SYNTHETIC_ISSUE_KEY: &str = "10519";

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

/// One data row's field values, mirroring the real stored `<td>` shapes.
/// `kind_issue_key` / `acceptance_number` are `Option`s so fail-closed tests
/// can render a row whose `onclick` id is entirely absent (`None`), not just
/// malformed (`Some("bad value")`).
struct RowInput<'a> {
    number: u64,
    time: &'a str,
    issue_name: &'a str,
    kind_issue_key: Option<&'a str>,
    disclosure_title: &'a str,
    acceptance_number: Option<&'a str>,
    filer_name: &'a str,
}

impl<'a> RowInput<'a> {
    /// A well-formed row for `번호=number`, with the other literal cell
    /// values supplied verbatim and both ids present and valid.
    fn well_formed(
        number: u64,
        time: &'a str,
        issue_name: &'a str,
        disclosure_title: &'a str,
        filer_name: &'a str,
    ) -> Self {
        Self {
            number,
            time,
            issue_name,
            kind_issue_key: Some(SYNTHETIC_ISSUE_KEY),
            disclosure_title,
            acceptance_number: Some(SYNTHETIC_ACCEPTANCE_NUMBER),
            filer_name,
        }
    }
}

/// Renders one `<tr>` of `<td>` cells, reproducing the real markup shapes:
/// an `<img>` + `<a title=... onclick="etfisusummary_open(...)">` in the
/// name cell, and an `<a title=... onclick="openDisclsViewer(...)">` in the
/// title cell. When an id is `None`, its `onclick` attribute is omitted
/// entirely from the anchor (simulating a row whose id is missing, not
/// merely malformed).
fn data_row(input: &RowInput<'_>) -> String {
    let name_onclick = match input.kind_issue_key {
        Some(key) => format!("onclick=\"etfisusummary_open('{key}'); return false;\""),
        None => String::new(),
    };
    let title_onclick = match input.acceptance_number {
        Some(no) => format!("onclick=\"openDisclsViewer('{no}','')\""),
        None => String::new(),
    };
    format!(
        r##"<tr class="first">
<td class="first txc" scope="row">{number}</td>
<td class="txc">{time}</td>
<td><img src='/images/common/icn_t_yu.gif' class='vmiddle legend' alt='유가증권'> <a id="etfisusum" href="#etfisusum" {name_onclick} title='{issue_name}'> {issue_name}</a> </td>
<td><a href="#viewer" {title_onclick} title='{disclosure_title}'>{disclosure_title}</a></td>
<td>{filer_name}</td>
</tr>"##,
        number = input.number,
        time = input.time,
        name_onclick = name_onclick,
        issue_name = input.issue_name,
        title_onclick = title_onclick,
        disclosure_title = input.disclosure_title,
        filer_name = input.filer_name,
    )
}

/// Convenience wrapper for the common case: a well-formed row rendered
/// directly to its `<tr>` HTML.
fn well_formed_row(number: u64, time: &str, issue_name: &str, title: &str, filer: &str) -> String {
    data_row(&RowInput::well_formed(
        number, time, issue_name, title, filer,
    ))
}

/// Assembles one page's full HTML fragment: the real KIND result table
/// shape — a `summary` attribute, an empty `<thead><tr>`, and `<tbody>` data
/// rows. No `<th>` cell anywhere, matching the real stored bytes.
fn page_html_with_placeholder(
    summary: &str,
    placeholder_row: Option<&str>,
    rows: &[String],
) -> Vec<u8> {
    let mut html = format!(
        r#"<table class="list type-00 tmt30" summary="{summary}">
<caption>목록</caption>
<colgroup></colgroup>
<thead>
"#
    );
    if let Some(placeholder_row) = placeholder_row {
        html.push_str(placeholder_row);
        html.push('\n');
    }
    html.push_str("</thead>\n<tbody>\n");
    for row in rows {
        html.push_str(row);
        html.push('\n');
    }
    html.push_str("</tbody>\n</table>");
    html.into_bytes()
}

fn page_html(summary: &str, rows: &[String]) -> Vec<u8> {
    page_html_with_placeholder(
        summary,
        Some(
            r#"<tr class="first" id="title-contents">
</tr>"#,
        ),
        rows,
    )
}

fn page_html_without_placeholder(summary: &str, rows: &[String]) -> Vec<u8> {
    page_html_with_placeholder(summary, None, rows)
}

/// A well-formed single page: the documented `summary` contract with the
/// given data rows.
fn well_formed_page(rows: &[String]) -> Vec<u8> {
    page_html(KIND_SUMMARY, rows)
}

/// Ingests `pages` (already-rendered HTML bytes, one per page) as a
/// `provider=kind-disclosure` Raw capture and returns the resulting source
/// manifest entry.
fn ingest_pages(store: &RawStore, pages: &[Vec<u8>]) -> ManifestEntry {
    let captured: Vec<CapturedPage> = pages
        .iter()
        .enumerate()
        .map(|(index, bytes)| CapturedPage {
            page_index: (index + 1) as u32,
            bytes: bytes.clone(),
            retrieved_at: fixed_retrieved_at(),
            form_fields: form_fields((index + 1) as u32),
        })
        .collect();
    ingest_disclosure_capture(
        store,
        MARKET_KR,
        &fixed_date(),
        SYNTHETIC_ENTITLEMENT_REFERENCE,
        FetchMode::Synthetic,
        KindSurface::EtfList,
        KindCaptureTermination::ClampedDuplicate,
        &captured,
    )
    .expect("well-formed capture ingest must succeed")
}

/// Ingests one single-page capture and returns its source manifest entry —
/// a shorthand for the many fail-closed tests that only need one page.
fn ingest_one_page(store: &RawStore, page: Vec<u8>) -> ManifestEntry {
    ingest_pages(store, &[page])
}

fn assert_nothing_normalized_was_written(store: &RawStore) {
    let manifest = store
        .read_manifest(PROVIDER_KIND_DISCLOSURE_NORMALIZED, MARKET_KR)
        .expect("reading an unwritten normalized manifest scope must not error");
    assert!(
        manifest.is_empty(),
        "expected no normalized batch to be written, found {manifest:?}"
    );
}

// ---------------------------------------------------------------------
// 1. Happy path.
// ---------------------------------------------------------------------

#[test]
fn happy_path_two_pages_parse_in_order_with_literal_fields_preserved() {
    let (_temp, store) = new_store();

    // Page 1: 15 rows, 번호 490 down to 476.
    let page1_rows: Vec<String> = (0..15)
        .map(|i| {
            let number = 490 - i as u64;
            well_formed_row(
                number,
                "2026-08-19 09:00",
                &format!("SYNTHETIC ETF {number}"),
                &format!("ETF 추가 ㆍ 변경상장신청서(수량변경)(일괄공시) {number}"),
                &format!("한국투자신탁운용 {number}"),
            )
        })
        .collect();
    // Page 2: 3 rows, 번호 475 down to 473 — the batch's partial last page.
    let page2_rows: Vec<String> = (0..3)
        .map(|i| {
            let number = 475 - i as u64;
            well_formed_row(
                number,
                "2026-08-19 09:05",
                &format!("SYNTHETIC ETF {number}"),
                &format!("ETF 추가 ㆍ 변경상장신청서(수량변경)(일괄공시) {number}"),
                &format!("한국투자신탁운용 {number}"),
            )
        })
        .collect();

    let source = ingest_pages(
        &store,
        &[well_formed_page(&page1_rows), well_formed_page(&page2_rows)],
    );

    let outcome =
        normalize_kind_disclosure_batch(&store, &source).expect("happy-path batch must normalize");

    assert_eq!(outcome.row_count, 18);
    assert_eq!(outcome.observations.len(), 18);
    assert_eq!(outcome.source_batch_id, source.batch_id);

    // Sequence numbers descend by exactly 1 across the whole batch, 490..=473.
    let numbers: Vec<u64> = outcome
        .observations
        .iter()
        .map(|observation| observation.sequence_number)
        .collect();
    let expected_numbers: Vec<u64> = (473..=490).rev().collect();
    assert_eq!(numbers, expected_numbers);

    // Literal fields are preserved exactly, in source page/row order.
    for observation in &outcome.observations {
        let number = observation.sequence_number;
        assert_eq!(observation.issue_name, format!("SYNTHETIC ETF {number}"));
        assert_eq!(
            observation.disclosure_title,
            format!("ETF 추가 ㆍ 변경상장신청서(수량변경)(일괄공시) {number}")
        );
        assert_eq!(observation.filer_name, format!("한국투자신탁운용 {number}"));
        // Both ids are extracted and preserved verbatim on every row.
        assert_eq!(observation.kind_internal_issue_key, SYNTHETIC_ISSUE_KEY);
        assert_eq!(
            observation.disclosure_acceptance_number,
            SYNTHETIC_ACCEPTANCE_NUMBER
        );
    }
    // First 15 came from page 1, in order, with page 1's literal timestamp;
    // last 3 from page 2, in order, with page 2's literal timestamp.
    for (index, observation) in outcome.observations.iter().take(15).enumerate() {
        assert_eq!(observation.source_file_name, "page-0001.html");
        assert_eq!(observation.source_row_index, index);
        assert_eq!(observation.posted_local_raw, "2026-08-19 09:00");
    }
    for (index, observation) in outcome.observations.iter().skip(15).enumerate() {
        assert_eq!(observation.source_file_name, "page-0002.html");
        assert_eq!(observation.source_row_index, index);
        assert_eq!(observation.posted_local_raw, "2026-08-19 09:05");
    }
}

// ---------------------------------------------------------------------
// 2. The recorded instant matches Asia/Seoul, and the assumption is
//    recorded.
// ---------------------------------------------------------------------

#[test]
fn recorded_instant_uses_the_explicit_asia_seoul_assumption() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        473,
        "2020-02-07 14:46",
        "ACE 200",
        "ETF 추가 ㆍ 변경상장신청서(수량변경)(일괄공시)",
        "한국투자신탁운용",
    )];
    let source = ingest_one_page(&store, well_formed_page(&rows));

    let outcome = normalize_kind_disclosure_batch(&store, &source).expect("must normalize");
    let observation = &outcome.observations[0];

    // The literal local value is preserved untouched.
    assert_eq!(observation.posted_local_raw, "2020-02-07 14:46");

    // The assumption is recorded explicitly, not silently baked in.
    assert_eq!(
        observation.timezone_assumption,
        TimezoneAssumption::AssumedAsiaSeoul
    );

    // 14:46 KST (UTC+09:00) is 05:46 UTC on the same calendar day.
    let expected_instant = UtcTimestamp::parse_rfc3339("2020-02-07T05:46:00Z").unwrap();
    assert_eq!(observation.posted_at_instant, expected_instant);
}

// ---------------------------------------------------------------------
// 3. Determinism.
// ---------------------------------------------------------------------

#[test]
fn normalizing_the_same_source_batch_twice_yields_the_same_normalized_batch_id() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        1,
        "2026-08-19 09:00",
        "SYNTHETIC ETF",
        "SYNTHETIC DISCLOSURE",
        "SYNTHETIC FILER",
    )];
    let source = ingest_one_page(&store, well_formed_page(&rows));

    let outcome_first = normalize_kind_disclosure_batch(&store, &source).expect("first call");
    let outcome_second = normalize_kind_disclosure_batch(&store, &source).expect("second call");

    assert_eq!(
        outcome_first.normalized_batch_id,
        outcome_second.normalized_batch_id
    );
    assert_eq!(
        outcome_first.normalized_batch_id,
        deterministic_kind_disclosure_normalized_batch_id(source.batch_id)
    );

    // Re-normalizing must not append a second manifest row.
    let manifest = store
        .read_manifest(PROVIDER_KIND_DISCLOSURE_NORMALIZED, MARKET_KR)
        .expect("read normalized manifest");
    assert_eq!(manifest.len(), 1);
}

// ---------------------------------------------------------------------
// 4. Fail-closed rules — one test per rule, each asserting a distinct
//    error and that nothing was written.
// ---------------------------------------------------------------------

#[test]
fn missing_summary_attribute_fails_closed() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        473,
        "2020-02-07 14:46",
        "ACE 200",
        "SYNTHETIC DISCLOSURE",
        "한국투자신탁운용",
    )];
    // A table with no `summary` attribute at all. The real page's
    // `fn_InitTitle(...)` script call *is* reproduced here (it is present
    // in real captured bytes too) to prove this module really does ignore
    // it as a contract source rather than falling back to it — and,
    // incidentally, to satisfy the Raw-capture ingest's own unrelated
    // `시간`-label presence check (see `providers::kind`), which this
    // fixture would otherwise fail before ever reaching the normalizer.
    let html = format!(
        r#"<table class="list type-00 tmt30">
<caption>목록</caption>
<thead>
<tr class="first" id="title-contents">
</tr>
</thead>
<tbody>
{}
</tbody>
</table>
<script language="javascript" type="text/JavaScript">
$(document).ready(function(){{
	fn_InitTitle("번호,시간,종목명,공시제목,제출인", "false,true,true,false,true");
}});
</script>"#,
        rows[0]
    );
    let source = ingest_one_page(&store, html.into_bytes());

    let error = normalize_kind_disclosure_batch(&store, &source)
        .expect_err("a page with no `summary` attribute must fail closed");
    match error {
        KindNormalizeError::UnsupportedHeader { actual, .. } => {
            assert!(
                actual.is_empty(),
                "a missing `summary` attribute must report an empty actual column list, got {actual:?}"
            );
        }
        other => panic!("expected UnsupportedHeader, got {other:?}"),
    }
    assert_nothing_normalized_was_written(&store);
}

#[test]
fn summary_listing_different_or_reordered_columns_fails_closed() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        473,
        "2020-02-07 14:46",
        "ACE 200",
        "SYNTHETIC DISCLOSURE",
        "한국투자신탁운용",
    )];
    for bad_summary in [
        // Reordered.
        "시간, 번호, 종목명, 공시제목, 제출인",
        // Missing 제출인, still containing 시간 so the Raw capture ingest
        // itself accepts the page (see providers::kind).
        "번호, 시간, 종목명, 공시제목",
        // A completely different column list.
        "번호, 시간, 종목코드, 종목명, 공시제목, 제출인",
    ] {
        let source = ingest_one_page(&store, page_html(bad_summary, &rows));
        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a non-conforming `summary` column list must fail closed");
        assert!(
            matches!(error, KindNormalizeError::UnsupportedHeader { .. }),
            "expected UnsupportedHeader for {bad_summary:?}, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn missing_placeholder_on_first_or_later_page_fails_closed() {
    let first_page_rows = [
        well_formed_row(
            490,
            "2026-08-19 09:00",
            "SYNTHETIC ETF 490",
            "SYNTHETIC DISCLOSURE 490",
            "SYNTHETIC FILER",
        ),
        well_formed_row(
            489,
            "2026-08-19 09:00",
            "SYNTHETIC ETF 489",
            "SYNTHETIC DISCLOSURE 489",
            "SYNTHETIC FILER",
        ),
    ];
    let preceding_page_rows = [well_formed_row(
        490,
        "2026-08-19 09:00",
        "SYNTHETIC ETF 490",
        "SYNTHETIC DISCLOSURE 490",
        "SYNTHETIC FILER",
    )];
    let later_page_rows = [well_formed_row(
        489,
        "2026-08-19 09:01",
        "SYNTHETIC ETF 489",
        "SYNTHETIC DISCLOSURE 489",
        "SYNTHETIC FILER",
    )];

    for pages in [
        vec![page_html_without_placeholder(
            KIND_SUMMARY,
            &first_page_rows,
        )],
        vec![
            well_formed_page(&preceding_page_rows),
            page_html_without_placeholder(KIND_SUMMARY, &later_page_rows),
        ],
    ] {
        let (_temp, store) = new_store();
        let source = ingest_pages(&store, &pages);
        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a missing placeholder must fail the entire batch");
        assert!(
            matches!(error, KindNormalizeError::InvalidPlaceholderRow { .. }),
            "expected InvalidPlaceholderRow, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn placeholder_with_td_or_th_cells_fails_closed() {
    let rows = [well_formed_row(
        490,
        "2026-08-19 09:00",
        "SYNTHETIC ETF 490",
        "SYNTHETIC DISCLOSURE 490",
        "SYNTHETIC FILER",
    )];
    for placeholder_row in [
        r#"<tr id="title-contents"><td></td></tr>"#,
        r#"<tr id="title-contents"><th></th></tr>"#,
        r#"<tr id="title-contents"><td>unterminated</tr>"#,
        r#"<tr id="title-contents"><th>unterminated</tr>"#,
    ] {
        let (_temp, store) = new_store();
        let source = ingest_one_page(
            &store,
            page_html_with_placeholder(KIND_SUMMARY, Some(placeholder_row), &rows),
        );
        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a placeholder containing a cell must fail the entire batch");
        assert!(
            matches!(error, KindNormalizeError::InvalidPlaceholderRow { .. }),
            "expected InvalidPlaceholderRow, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn row_with_wrong_cell_count_fails_closed() {
    let (_temp, store) = new_store();
    let row = r#"<tr class="first">
<td class="first txc" scope="row">473</td>
<td class="txc">2020-02-07 14:46</td>
<td><a onclick="etfisusummary_open('10519'); return false;" title='ACE 200'>ACE 200</a></td>
<td><a onclick="openDisclsViewer('20200207000058','')" title='SYNTHETIC DISCLOSURE'>SYNTHETIC DISCLOSURE</a></td>
</tr>"#
        // Missing 제출인 cell: only 4 cells in a 5-column table.
        .to_owned();
    let source = ingest_one_page(&store, well_formed_page(&[row]));

    let error = normalize_kind_disclosure_batch(&store, &source)
        .expect_err("a row with the wrong cell count must fail closed");
    assert!(
        matches!(error, KindNormalizeError::RowCellCountMismatch { .. }),
        "expected RowCellCountMismatch, got {error:?}"
    );
    assert_nothing_normalized_was_written(&store);
}

#[test]
fn unparseable_time_fails_closed() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        473,
        "2020/02/07 14:46", // wrong separators
        "ACE 200",
        "SYNTHETIC DISCLOSURE",
        "한국투자신탁운용",
    )];
    let source = ingest_one_page(&store, well_formed_page(&rows));

    let error = normalize_kind_disclosure_batch(&store, &source)
        .expect_err("a malformed 시간 value must fail closed");
    assert!(
        matches!(error, KindNormalizeError::InvalidTimestamp { .. }),
        "expected InvalidTimestamp, got {error:?}"
    );
    assert_nothing_normalized_was_written(&store);
}

#[test]
fn empty_issue_name_or_title_fails_closed() {
    // Table-driven: an empty 종목명 and an empty 공시제목 are the same
    // documented rule, each asserted as the same distinct error variant.
    let cases: [(&str, &str, RequiredField); 2] = [
        ("", "SYNTHETIC DISCLOSURE", RequiredField::IssueName),
        ("ACE 200", "", RequiredField::DisclosureTitle),
    ];
    for (issue_name, title, expected_field) in cases {
        let (_temp, store) = new_store();
        let rows = [well_formed_row(
            473,
            "2020-02-07 14:46",
            issue_name,
            title,
            "한국투자신탁운용",
        )];
        let source = ingest_one_page(&store, well_formed_page(&rows));

        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("an empty required field must fail closed");
        match error {
            KindNormalizeError::EmptyRequiredField { field, .. } => {
                assert_eq!(field, expected_field);
            }
            other => panic!("expected EmptyRequiredField, got {other:?}"),
        }
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn non_positive_sequence_number_fails_closed() {
    for bad_number in ["0", "abc", "-5", ""] {
        let (_temp, store) = new_store();
        let row = format!(
            r#"<tr class="first">
<td class="first txc" scope="row">{bad_number}</td>
<td class="txc">2020-02-07 14:46</td>
<td><a onclick="etfisusummary_open('10519'); return false;" title='ACE 200'>ACE 200</a></td>
<td><a onclick="openDisclsViewer('20200207000058','')" title='SYNTHETIC DISCLOSURE'>SYNTHETIC DISCLOSURE</a></td>
<td>한국투자신탁운용</td>
</tr>"#
        );
        let source = ingest_one_page(&store, well_formed_page(&[row]));

        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a non-positive-integer 번호 must fail closed");
        assert!(
            matches!(error, KindNormalizeError::InvalidSequenceNumber { .. }),
            "expected InvalidSequenceNumber for {bad_number:?}, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn sequence_number_gap_across_pages_fails_closed() {
    let (_temp, store) = new_store();
    // Page 1 ends at 461; page 2 starts at 459 — a gap (460 missing) that
    // only shows up once the whole batch is considered together.
    let page1_rows = [well_formed_row(
        461,
        "2026-08-19 09:00",
        "SYNTHETIC ETF A",
        "SYNTHETIC DISCLOSURE A",
        "SYNTHETIC FILER",
    )];
    let page2_rows = [well_formed_row(
        459,
        "2026-08-19 09:05",
        "SYNTHETIC ETF B",
        "SYNTHETIC DISCLOSURE B",
        "SYNTHETIC FILER",
    )];
    let source = ingest_pages(
        &store,
        &[well_formed_page(&page1_rows), well_formed_page(&page2_rows)],
    );

    let error = normalize_kind_disclosure_batch(&store, &source)
        .expect_err("a 번호 gap across pages must fail closed");
    assert!(
        matches!(error, KindNormalizeError::SequenceNumberOutOfOrder { .. }),
        "expected SequenceNumberOutOfOrder, got {error:?}"
    );
    assert_nothing_normalized_was_written(&store);
}

#[test]
fn zero_rows_across_the_whole_batch_fails_closed() {
    let (_temp, store) = new_store();
    // The `<thead>` placeholder row only, no data rows on the batch's
    // single page.
    let source = ingest_one_page(&store, well_formed_page(&[]));

    let error = normalize_kind_disclosure_batch(&store, &source)
        .expect_err("zero rows across the whole batch must fail closed");
    assert!(
        matches!(error, KindNormalizeError::EmptyBatch),
        "expected EmptyBatch, got {error:?}"
    );
    assert_nothing_normalized_was_written(&store);
}

#[test]
fn missing_or_malformed_kind_internal_issue_key_fails_closed() {
    for kind_issue_key in [None, Some(""), Some("12x45"), Some("1234567890123")] {
        let (_temp, store) = new_store();
        let row = data_row(&RowInput {
            number: 473,
            time: "2020-02-07 14:46",
            issue_name: "ACE 200",
            kind_issue_key,
            disclosure_title: "SYNTHETIC DISCLOSURE",
            acceptance_number: Some(SYNTHETIC_ACCEPTANCE_NUMBER),
            filer_name: "한국투자신탁운용",
        });
        let source = ingest_one_page(&store, well_formed_page(&[row]));

        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a missing or malformed KIND-internal issue key must fail closed");
        assert!(
            matches!(
                error,
                KindNormalizeError::InvalidKindInternalIssueKey { .. }
            ),
            "expected InvalidKindInternalIssueKey for {kind_issue_key:?}, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

#[test]
fn missing_or_malformed_disclosure_acceptance_number_fails_closed() {
    for acceptance_number in [
        None,
        Some(""),
        Some("2020020714460"),   // 13 digits, one short
        Some("202002071446000"), // 15 digits, one too many
        Some("2020020714460x"),  // non-digit
    ] {
        let (_temp, store) = new_store();
        let row = data_row(&RowInput {
            number: 473,
            time: "2020-02-07 14:46",
            issue_name: "ACE 200",
            kind_issue_key: Some(SYNTHETIC_ISSUE_KEY),
            disclosure_title: "SYNTHETIC DISCLOSURE",
            acceptance_number,
            filer_name: "한국투자신탁운용",
        });
        let source = ingest_one_page(&store, well_formed_page(&[row]));

        let error = normalize_kind_disclosure_batch(&store, &source)
            .expect_err("a missing or malformed disclosure acceptance number must fail closed");
        assert!(
            matches!(
                error,
                KindNormalizeError::InvalidDisclosureAcceptanceNumber { .. }
            ),
            "expected InvalidDisclosureAcceptanceNumber for {acceptance_number:?}, got {error:?}"
        );
        assert_nothing_normalized_was_written(&store);
    }
}

// ---------------------------------------------------------------------
// 5. The disclosure acceptance number is preserved verbatim, and no date
//    is ever derived from it.
// ---------------------------------------------------------------------

#[test]
fn acceptance_number_is_preserved_verbatim_and_no_date_is_derived_from_it() {
    let (_temp, store) = new_store();
    // Deliberately chosen so the acceptance number's leading 8 digits
    // decode to a *different* calendar date (2018-01-01) than the row's own
    // 시간 date (2020-02-07): if any code path silently derived a date from
    // the acceptance number, this mismatch would surface it immediately.
    let mismatched_acceptance_number = "20180101000058";
    let row = data_row(&RowInput {
        number: 473,
        time: "2020-02-07 14:46",
        issue_name: "ACE 200",
        kind_issue_key: Some(SYNTHETIC_ISSUE_KEY),
        disclosure_title: "SYNTHETIC DISCLOSURE",
        acceptance_number: Some(mismatched_acceptance_number),
        filer_name: "한국투자신탁운용",
    });
    let source = ingest_one_page(&store, well_formed_page(&[row]));

    let outcome = normalize_kind_disclosure_batch(&store, &source).expect("must normalize");
    let observation = &outcome.observations[0];

    // Preserved verbatim, as a plain opaque string.
    assert_eq!(
        observation.disclosure_acceptance_number,
        mismatched_acceptance_number
    );

    // No field on the observation equals a date parsed out of its leading
    // digits: `posted_local`'s date remains exactly the 시간 cell's date,
    // never overwritten by (or reconciled with) the acceptance number.
    let posted_date = observation.posted_local.date();
    let acceptance_number_leading_date =
        chrono::NaiveDate::parse_from_str(&mismatched_acceptance_number[0..8], "%Y%m%d")
            .expect("the leading 8 digits happen to be a valid calendar date");
    assert_ne!(
        posted_date, acceptance_number_leading_date,
        "posted_local's date must never be derived from the acceptance number"
    );
    assert_eq!(
        observation.posted_local_raw, "2020-02-07 14:46",
        "the literal 시간 cell is untouched by the acceptance number's content"
    );
}

// ---------------------------------------------------------------------
// 6. Instrument identity is exposed as unresolved, not merely absent.
// ---------------------------------------------------------------------

#[test]
fn instrument_identity_is_exposed_as_unresolved_not_forgotten() {
    let (_temp, store) = new_store();
    let rows = [well_formed_row(
        473,
        "2020-02-07 14:46",
        "ACE 200",
        "ETF 추가 ㆍ 변경상장신청서(수량변경)(일괄공시)",
        "한국투자신탁운용",
    )];
    let source = ingest_one_page(&store, well_formed_page(&rows));

    let outcome = normalize_kind_disclosure_batch(&store, &source).expect("must normalize");
    let observation = &outcome.observations[0];

    // The literal name is preserved byte-faithfully...
    assert_eq!(observation.issue_name, "ACE 200");
    // ...and the KIND-internal issue key is preserved too, but it is not an
    // instrument id either.
    assert_eq!(observation.kind_internal_issue_key, SYNTHETIC_ISSUE_KEY);
    // ...but resolving either to an instrument id is explicitly blocked (no
    // authoritative KIND name/key-to-code mapping exists in this
    // repository, pending the deferred KRX decision, and the stored bytes
    // themselves carry no six-digit KRX code anywhere), not simply omitted
    // by oversight — the type itself carries this state rather than
    // leaving the field absent.
    match &observation.instrument_identity {
        InstrumentIdentity::Unresolved { reason } => {
            assert!(
                !reason.is_empty(),
                "the unresolved marker must explain why, not just that"
            );
        }
    }
}
