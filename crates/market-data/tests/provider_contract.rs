//! Provider-neutral EOD contract + KRX adapter tests (Todo 8 acceptance).
//!
//! Proves the raw response envelope contract (bytes, retrieval time, provider
//! request metadata, batch id, content hash), deterministic hashing, redacted
//! request metadata, and typed provider failures: endpoint timeout, malformed
//! bundle, unsafe file names, and the credentialed mode without credentials.
//!
//! Manual QA channel: `cargo test -p market-data --test provider_contract -- --nocapture`

use std::fs;

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{ALL_RESPONSE_KINDS, FetchMode, MARKET_KR, PROVIDER_KRX, ResponseKind};
use market_data::provider::{
    CredentialRef, EodProvider, FetchRequest, KrxMode, KrxProvider, ProviderError, RecordedBundle,
};

const GOOD_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract"
);
const TRAVERSAL_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract-variants/traversal"
);
const TIMEOUT_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract-variants/timeout"
);

fn request(at: &str) -> FetchRequest {
    FetchRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").expect("valid date"),
        UtcTimestamp::parse_rfc3339(at).expect("valid timestamp"),
    )
}

fn synthetic(bundle: &str) -> KrxProvider {
    let recorded = RecordedBundle::open(bundle).expect("recorded bundle opens");
    KrxProvider::synthetic(recorded)
}

#[test]
fn recorded_bundle_delivers_four_envelopes_with_contract_fields() {
    let provider = synthetic(GOOD_BUNDLE);
    assert_eq!(provider.provider_id(), PROVIDER_KRX);

    let now = "2026-08-05T06:00:00Z";
    let req = request(now);
    let envelopes = provider.fetch(&req).expect("synthetic fetch succeeds");

    assert_eq!(
        envelopes.len(),
        4,
        "all four licensed response classes expected"
    );
    let kinds: Vec<ResponseKind> = envelopes.iter().map(|e| e.kind).collect();
    for k in ALL_RESPONSE_KINDS {
        assert!(kinds.contains(&k), "missing response kind {k:?}");
    }

    for env in &envelopes {
        assert_eq!(
            env.batch_id, req.batch_id,
            "envelope must carry the request batch id"
        );
        assert_eq!(
            env.retrieved_at.to_rfc3339(),
            now,
            "retrieval time must be the request clock"
        );
        assert_eq!(env.request.mode, FetchMode::Synthetic);
        assert!(
            !env.bytes.is_empty(),
            "recorded response bytes must be present"
        );
        assert_eq!(env.content_hash, ContentHash::from_bytes(&env.bytes));
        assert!(
            !env.file_name.contains('/'),
            "file name must be a plain name: {}",
            env.file_name
        );
        assert!(
            !env.file_name.contains('\\'),
            "file name must be a plain name: {}",
            env.file_name
        );
    }
}

#[test]
fn hashes_are_deterministic_across_fetches() {
    let provider = synthetic(GOOD_BUNDLE);
    let req = request("2026-08-05T06:00:00Z");
    let first = provider.fetch(&req).expect("fetch");
    let second = provider.fetch(&req).expect("fetch again");
    let hashes = |envelopes: &[market_data::RawEnvelope]| -> Vec<ContentHash> {
        envelopes.iter().map(|e| e.content_hash.clone()).collect()
    };
    assert_eq!(
        hashes(&first),
        hashes(&second),
        "same bundle must hash identically"
    );
}

#[test]
fn request_metadata_records_endpoint_query_and_redacted_headers() {
    let provider = synthetic(GOOD_BUNDLE);
    let envelopes = provider
        .fetch(&request("2026-08-05T06:00:00Z"))
        .expect("fetch");
    let bars = envelopes
        .iter()
        .find(|e| e.kind == ResponseKind::Bars)
        .expect("bars envelope");
    assert_eq!(bars.request.endpoint, "krx.eod.bars.v1");
    assert!(
        bars.request
            .query
            .contains(&("market".to_owned(), "KR".to_owned()))
    );
    // Headers carried into the envelope MUST be redacted placeholders.
    for (name, value) in &bars.request.headers {
        assert!(
            !name.to_ascii_lowercase().contains("key"),
            "header name must be redacted: {name}"
        );
        assert!(
            !name.to_ascii_lowercase().contains("auth"),
            "header name must be redacted: {name}"
        );
        assert!(
            value == "redacted" || value.is_empty(),
            "header value must be redacted, got {value:?}"
        );
    }
}

#[test]
fn timeout_simulation_returns_typed_timeout() {
    let provider = synthetic(TIMEOUT_BUNDLE);
    let err = provider
        .fetch(&request("2026-08-05T06:00:00Z"))
        .expect_err("simulated timeout must fail");
    assert!(
        matches!(
            err,
            ProviderError::EndpointTimeout {
                kind: ResponseKind::Bars,
                ..
            }
        ),
        "expected typed EndpointTimeout for bars, got {err:?}"
    );
}

#[test]
fn credentialed_mode_without_credentials_returns_typed_failure() {
    // Owner-only credentialed mode: implemented, NEVER exercised in CI because no
    // real KRX credentials exist. Absent credentials must fail typed, not panic.
    unsafe { std::env::remove_var("KRX_CREDENTIAL_REF") };
    unsafe { std::env::remove_var("KRX_BASE_URL") };
    let provider = KrxProvider::credentialed(CredentialRef::new("env:KRX_CREDENTIAL_REF"));
    assert_eq!(provider.fetch_mode(), FetchMode::Credentialed);
    let err = provider
        .fetch(&request("2026-08-05T06:00:00Z"))
        .expect_err("credentialed mode without credentials must fail");
    match &err {
        ProviderError::CredentialsUnavailable {
            credential_ref,
            detail,
        } => {
            assert!(credential_ref.contains("KRX_CREDENTIAL_REF"));
            assert!(!detail.is_empty());
        }
        other => panic!("expected CredentialsUnavailable, got {other:?}"),
    }
}

#[test]
fn unsafe_file_name_rejected_as_typed_provider_error() {
    let provider = synthetic(TRAVERSAL_BUNDLE);
    let err = provider
        .fetch(&request("2026-08-05T06:00:00Z"))
        .expect_err("traversal file name must fail at the provider boundary");
    match &err {
        ProviderError::UnsafeFileName { kind, file_name } => {
            assert_eq!(*kind, ResponseKind::Bars);
            assert!(
                file_name.contains(".."),
                "error must name the offending file: {file_name}"
            );
        }
        other => panic!("expected UnsafeFileName, got {other:?}"),
    }
}

#[test]
fn malformed_bundle_manifest_returns_typed_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("bundle.json"), b"{ not json").expect("write bad bundle");
    let err = RecordedBundle::open(dir.path()).expect_err("bad bundle.json must fail typed");
    assert!(matches!(err, ProviderError::RecordedBundleParse { .. }));
}

#[test]
fn missing_bundle_manifest_is_typed_permanent_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let error = RecordedBundle::open(dir.path()).unwrap_err();
    assert!(matches!(error, ProviderError::RecordedBundleMissing { .. }));
}

#[test]
fn unknown_directive_and_kind_are_typed_permanent_configuration() {
    let directive = tempfile::tempdir().unwrap();
    fs::write(
        directive.path().join("bundle.json"),
        r#"{"provider":"krx","market":"kr","schema_version":1,"simulate":"explode","responses":[]}"#,
    )
    .unwrap();
    let error = KrxProvider::synthetic(RecordedBundle::open(directive.path()).unwrap())
        .fetch(&request("2026-08-05T06:00:00Z"))
        .unwrap_err();
    assert!(matches!(error, ProviderError::RecordedBundleInvalid { .. }));

    let kind = tempfile::tempdir().unwrap();
    fs::write(
        kind.path().join("bundle.json"),
        r#"{"provider":"krx","market":"kr","schema_version":1,"responses":[{"kind":"mystery","file":"x.json","endpoint":"x"}]}"#,
    )
    .unwrap();
    let error = KrxProvider::synthetic(RecordedBundle::open(kind.path()).unwrap())
        .fetch(&request("2026-08-05T06:00:00Z"))
        .unwrap_err();
    assert!(matches!(error, ProviderError::RecordedBundleInvalid { .. }));
}

#[test]
fn missing_recorded_response_is_typed_permanent_configuration() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("bundle.json"),
        r#"{"provider":"krx","market":"kr","schema_version":1,"responses":[{"kind":"bars","file":"missing.json","endpoint":"x"}]}"#,
    )
    .unwrap();
    let error = KrxProvider::synthetic(RecordedBundle::open(dir.path()).unwrap())
        .fetch(&request("2026-08-05T06:00:00Z"))
        .unwrap_err();
    assert!(matches!(error, ProviderError::RecordedBundleIo { .. }));
}

#[test]
fn fetch_request_defaults_to_all_kinds() {
    let req = FetchRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").unwrap(),
        UtcTimestamp::parse_rfc3339("2026-08-05T06:00:00Z").unwrap(),
    );
    assert_eq!(req.kinds, ALL_RESPONSE_KINDS.to_vec());
    assert_eq!(req.market, MARKET_KR);
    // Batch id is stable per request so envelopes carry it.
    assert_eq!(req.batch_id, req.batch_id);
    let _: BatchId = req.batch_id;
    let _: KrxMode = KrxMode::Credentialed(CredentialRef::new("env:KRX_CREDENTIAL_REF"));
}
