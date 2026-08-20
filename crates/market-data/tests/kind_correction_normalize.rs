use domain::{TradingDate, UtcTimestamp};
use market_data::contract::{
    PROVIDER_KIND_DISCLOSURE_CORRECTION, PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED,
};
use market_data::providers::kind::{
    KIND_CORRECTION_ARTIFACT_KIND, KIND_CORRECTION_ENTRY_URL, KIND_CORRECTION_SURFACE,
    KIND_CORRECTION_TERMINATION, KIND_CORRECTION_TERMINATION_STAGE, KIND_CORRECTION_VIEWER_FILE,
    KIND_CORRECTION_VIEWER_ORIGIN_PATH,
};
use market_data::{
    FetchMode, KindCorrectionCapture, KindCorrectionResponseDiagnostics, KindCorrectionViewerError,
    MARKET_KR, RawStore, ResponseKind, ingest_correction_capture, normalize_kind_correction_batch,
    parse_kind_correction_membership,
};

const ANCHOR: &str = "20200207000058";

fn viewer(options: &str) -> Vec<u8> {
    format!("<html><body><select id=\"mainDoc\" name=\"mainDoc\">{options}</select></body></html>")
        .into_bytes()
}

fn happy_options() -> &'static str {
    "<option value=\"\"></option>\
     <option value=\"20200207000081|Y\">2020.02.07 정정</option>\
     <option value=\"20200207000082|Y\">2020.02.08 원문</option>"
}

fn capture(bytes: Vec<u8>) -> KindCorrectionCapture {
    KindCorrectionCapture {
        source: "kind.krx.co.kr".to_owned(),
        entry_url: KIND_CORRECTION_ENTRY_URL.to_owned(),
        surface: KIND_CORRECTION_SURFACE.to_owned(),
        requested_from: TradingDate::parse("2020-02-03").unwrap(),
        requested_to: TradingDate::parse("2020-02-07").unwrap(),
        anchor_acceptance_number: ANCHOR.to_owned(),
        viewer_origin_path: KIND_CORRECTION_VIEWER_ORIGIN_PATH.to_owned(),
        artifact_kind: KIND_CORRECTION_ARTIFACT_KIND.to_owned(),
        retrieved_at: UtcTimestamp::parse_rfc3339("2026-08-20T00:00:00Z").unwrap(),
        termination: KIND_CORRECTION_TERMINATION.to_owned(),
        termination_stage: KIND_CORRECTION_TERMINATION_STAGE.to_owned(),
        response_diagnostics: KindCorrectionResponseDiagnostics {
            body_size: 12_852,
            form_field_count: 5,
            target_handler_occurrences: 1,
        },
        file_name: KIND_CORRECTION_VIEWER_FILE.to_owned(),
        viewer_bytes: bytes,
    }
}

#[test]
fn parses_one_entry_without_claiming_anchor_membership() {
    let bytes = viewer(
        "<option value=\"\">공시문서 선택</option><option value=\"20200207000081|Y\">2020.02.07 정정</option>",
    );
    let membership = parse_kind_correction_membership(&bytes, ANCHOR).unwrap();
    assert_eq!(membership.anchor_acceptance_number, ANCHOR);
    assert_eq!(membership.ordered_versions.len(), 1);
    assert_eq!(
        membership.ordered_versions[0].acceptance_number,
        "20200207000081"
    );
    assert_eq!(membership.ordered_versions[0].raw_value, "20200207000081|Y");
    assert_eq!(membership.ordered_versions[0].date_literal, "2020.02.07");
    assert_eq!(membership.ordered_versions[0].label, "2020.02.07 정정");
}

#[test]
fn preserves_multi_entry_rendered_order() {
    let membership = parse_kind_correction_membership(&viewer(happy_options()), ANCHOR).unwrap();
    let values: Vec<_> = membership
        .ordered_versions
        .iter()
        .map(|entry| entry.acceptance_number.as_str())
        .collect();
    assert_eq!(values, ["20200207000081", "20200207000082"]);
    assert_eq!(membership.ordered_versions[0].option_index, 1);
    assert_eq!(membership.ordered_versions[1].option_index, 2);
}

#[test]
fn malformed_viewer_shapes_fail_closed() {
    let cases = [
        (
            "<select id=\"mainDoc\"><option value=\"\"></option></select>",
            "missing version",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"x\"></option><option value=\"20200207000081|Y\">2020.02.07</option></select>",
            "placeholder",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"\"></option><option value=\"20200207000081|N\">2020.02.07</option></select>",
            "N",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"\"></option><option value=\"20200207000081|Y\">2020.02.31</option></select>",
            "calendar date",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"\"></option><option value=\"20200207000081|Y\">2020.02.07 2020.02.07</option></select>",
            "duplicate date",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"\"></option><option value=\"20200207000081|Y\">missing date</option></select>",
            "date token",
        ),
        (
            "<select id=\"mainDoc\"><option value=\"\"></option><option value=\"20200207000081|Y\">2020.02.07</option><option value=\"20200207000081|Y\">2020.02.08</option></select>",
            "duplicate acceptance",
        ),
        (
            "<select id=\"mainDoc\"><option></option><option value=\"20200207000081|Y\">2020.02.07</option></select>",
            "absent placeholder value",
        ),
    ];
    for (html, _label) in cases {
        assert!(parse_kind_correction_membership(html.as_bytes(), ANCHOR).is_err());
    }
    assert!(matches!(
        parse_kind_correction_membership(&[0xff], ANCHOR),
        Err(KindCorrectionViewerError::MalformedUtf8)
    ));
}

#[test]
fn raw_and_normalized_scopes_are_distinct_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let store = RawStore::new(temp.path());
    let date = TradingDate::parse("2026-08-20").unwrap();
    let raw = ingest_correction_capture(
        &store,
        MARKET_KR,
        &date,
        "fixture://kind-correction",
        FetchMode::Synthetic,
        &capture(viewer(happy_options())),
    )
    .unwrap();
    assert_eq!(raw.provider, PROVIDER_KIND_DISCLOSURE_CORRECTION);
    assert_eq!(raw.files[0].kind, ResponseKind::DisclosureVersionMembership);
    let normalized = normalize_kind_correction_batch(&store, &raw).unwrap();
    assert_eq!(
        normalized.normalized_provider,
        PROVIDER_KIND_DISCLOSURE_CORRECTION_NORMALIZED
    );
    assert_eq!(normalized.membership.ordered_versions.len(), 2);
    let replay = normalize_kind_correction_batch(&store, &raw).unwrap();
    assert_eq!(replay.normalized_batch_id, normalized.normalized_batch_id);
}

#[test]
fn raw_rejects_oversized_response_diagnostic_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let store = RawStore::new(temp.path());
    let date = TradingDate::parse("2026-08-20").unwrap();
    let mut invalid = capture(viewer(happy_options()));
    invalid.response_diagnostics.body_size = 1024 * 1024 + 1;
    assert!(
        ingest_correction_capture(
            &store,
            MARKET_KR,
            &date,
            "fixture://kind-correction",
            FetchMode::Synthetic,
            &invalid,
        )
        .is_err()
    );
    assert!(
        !store
            .provider_dir(PROVIDER_KIND_DISCLOSURE_CORRECTION, MARKET_KR)
            .exists()
    );
    assert!(
        !store
            .manifest_path(PROVIDER_KIND_DISCLOSURE_CORRECTION, MARKET_KR)
            .exists()
    );
}
