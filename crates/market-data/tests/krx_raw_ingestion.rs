//! End-to-end KRX raw ingestion tests (Todo 8 acceptance test target:
//! `cargo test -p market-data --test krx_raw_ingestion`).
//!
//! Proves the full collector pipeline against the recorded synthetic bundle:
//! - one delivery => one immutable batch under `data/raw/provider=krx/market=kr/date=...`;
//! - identical bytes delivered twice => TWO batches, same content hash, first untouched;
//! - append-only manifest (one row per batch);
//! - timeout / malformed schema / path traversal => typed failure with NO partial output;
//! - an ACTIVE entitlement record is referenced on the manifest row.
//!
//! Manual QA channel: `cargo test -p market-data --test krx_raw_ingestion -- --nocapture`

use std::fs;

use auth::entitlement::{
    CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash, Entitlement, EntitlementId,
    EntitlementService, EntitlementState, KrUseRegistry, UserId,
};
use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{FetchMode, MARKET_KR, PROVIDER_KRX, ResponseKind};
use market_data::ingest::{IngestError, IngestRequest, ingest_bundle};
use market_data::provider::{KrxProvider, ProviderError, RecordedBundle};
use market_data::storage::{RawStore, StoreError};

const GOOD_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract"
);
const MALFORMED_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract-variants/malformed-bars"
);
const TRAVERSAL_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract-variants/traversal"
);
const TIMEOUT_BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/kr-etf/contract-variants/timeout"
);

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ls-task8-ing-{tag}-{}", BatchId::generate()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn request(at: &str) -> IngestRequest {
    IngestRequest::new(
        MARKET_KR.to_owned(),
        TradingDate::parse("2020-01-31").expect("valid date"),
        UtcTimestamp::parse_rfc3339(at).expect("valid timestamp"),
    )
}

fn active_entitlement_service() -> EntitlementService {
    EntitlementService::new(vec![
        Entitlement::builder()
            .id(EntitlementId::new("ent_krx_2026_0001"))
            .provider(DataProvider::Krx)
            .contract(ContractRef::new(
                DocumentHash::sha256("aa".repeat(32)),
                "vault://krx-entitlements/ent_krx_2026_0001.pdf",
            ))
            .lifecycle(EntitlementState::Active)
            .effective(
                CalendarDate::parse("2019-01-01").expect("from"),
                CalendarDate::parse("2026-12-31").expect("until"),
            )
            .covered_datasets([DatasetId::krx_eod_bars()])
            .covered_uses(KrUseRegistry::standard().member_visible().to_vec())
            .covered_users([UserId::new("usr_a")])
            .build(),
    ])
}

#[test]
fn ingest_synthetic_bundle_creates_immutable_batch() {
    let root = temp_root("single");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(GOOD_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let at = "2026-08-05T07:00:00Z";
    let req = request(at);

    let outcome = ingest_bundle(&store, &provider, &req, None).expect("ingest succeeds");

    assert_eq!(outcome.entry.provider, PROVIDER_KRX);
    assert_eq!(outcome.entry.market, MARKET_KR);
    assert_eq!(outcome.entry.date.to_iso(), "2020-01-31");
    assert_eq!(outcome.entry.retrieved_at.to_rfc3339(), at);
    assert_eq!(outcome.entry.mode, FetchMode::Synthetic);
    assert_eq!(outcome.entry.entitlement_reference, None);
    assert_eq!(outcome.entry.files.len(), 4);
    assert_eq!(outcome.files.len(), 4);
    assert_eq!(outcome.batch_id, outcome.entry.batch_id);

    // One batch dir under the documented date partition; manifest has one row.
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &req.date)
        .expect("list");
    assert_eq!(dirs, vec![outcome.batch_id]);
    let manifest = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest");
    assert_eq!(manifest.len(), 1);
    assert_eq!(manifest[0], outcome.entry);

    // Stored bytes are byte-identical to the recorded provider files.
    let back = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &outcome.entry)
        .expect("read back");
    for file in &back {
        let fixture = fs::read(std::path::Path::new(GOOD_BUNDLE).join(&file.file_name))
            .expect("fixture readable");
        assert_eq!(
            file.bytes, fixture,
            "stored bytes must equal recorded provider bytes: {}",
            file.file_name
        );
    }
}

#[test]
fn duplicate_delivery_two_batches_same_hash_first_untouched() {
    let root = temp_root("dup");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(GOOD_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let req = request("2026-08-05T07:00:00Z");

    let first = ingest_bundle(&store, &provider, &req, None).expect("first delivery");
    let second = ingest_bundle(&store, &provider, &req, None).expect("duplicate delivery");

    assert_ne!(
        first.batch_id, second.batch_id,
        "duplicate delivery must create a NEW batch"
    );
    let h1: Vec<ContentHash> = first
        .entry
        .files
        .iter()
        .map(|f| f.content_hash.clone())
        .collect();
    let h2: Vec<ContentHash> = second
        .entry
        .files
        .iter()
        .map(|f| f.content_hash.clone())
        .collect();
    assert_eq!(
        h1, h2,
        "identical bytes must produce identical content hashes"
    );

    // First batch untouched.
    let back1 = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &first.entry)
        .expect("first read");
    for file in &back1 {
        let fixture =
            fs::read(std::path::Path::new(GOOD_BUNDLE).join(&file.file_name)).expect("fixture");
        assert_eq!(
            file.bytes, fixture,
            "first batch must be untouched: {}",
            file.file_name
        );
    }

    // Two rows in the append-only manifest, two batch dirs.
    let manifest = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest");
    assert_eq!(manifest.len(), 2, "manifest must hold one row per delivery");
    assert_eq!(manifest[0], first.entry);
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &req.date)
        .expect("list");
    assert_eq!(dirs.len(), 2);
    assert!(dirs.contains(&first.batch_id));
    assert!(dirs.contains(&second.batch_id));

    // Manifest file on disk: exactly two lines (append-only JSONL).
    let raw =
        fs::read_to_string(store.manifest_path(PROVIDER_KRX, MARKET_KR)).expect("manifest file");
    assert_eq!(raw.lines().count(), 2);
}

#[test]
fn malformed_response_typed_failure_with_no_partial_output() {
    let root = temp_root("malformed");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(MALFORMED_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let req = request("2026-08-05T07:00:00Z");

    let err = ingest_bundle(&store, &provider, &req, None).expect_err("malformed schema must fail");
    match &err {
        IngestError::MalformedResponse { kind, reason } => {
            assert_eq!(*kind, ResponseKind::Bars);
            assert!(!reason.is_empty());
        }
        other => panic!("expected typed MalformedResponse, got {other:?}"),
    }

    // No partial curated/batch output: no batch dirs, empty manifest.
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &req.date)
        .expect("list");
    assert!(
        dirs.is_empty(),
        "malformed delivery must leave no batch dir: {dirs:?}"
    );
    assert!(
        store
            .read_manifest(PROVIDER_KRX, MARKET_KR)
            .expect("manifest")
            .is_empty()
    );
}

#[test]
fn timeout_typed_failure_with_no_partial_output() {
    let root = temp_root("timeout");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(TIMEOUT_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let req = request("2026-08-05T07:00:00Z");

    let err = ingest_bundle(&store, &provider, &req, None).expect_err("timeout must fail");
    assert!(
        matches!(
            err,
            IngestError::Provider(ProviderError::EndpointTimeout {
                kind: ResponseKind::Bars,
                ..
            })
        ),
        "expected typed EndpointTimeout via Provider, got {err:?}"
    );
    assert!(
        store
            .batch_ids(PROVIDER_KRX, MARKET_KR, &req.date)
            .expect("list")
            .is_empty()
    );
    assert!(
        store
            .read_manifest(PROVIDER_KRX, MARKET_KR)
            .expect("manifest")
            .is_empty()
    );
}

#[test]
fn path_traversal_typed_failure_with_no_partial_output() {
    let root = temp_root("traversal");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(TRAVERSAL_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let req = request("2026-08-05T07:00:00Z");

    let err = ingest_bundle(&store, &provider, &req, None).expect_err("traversal must fail");
    assert!(
        matches!(
            err,
            IngestError::Provider(ProviderError::UnsafeFileName { .. })
        ),
        "expected typed UnsafeFileName via Provider, got {err:?}"
    );
    assert!(
        store
            .batch_ids(PROVIDER_KRX, MARKET_KR, &req.date)
            .expect("list")
            .is_empty()
    );
    assert!(
        store
            .read_manifest(PROVIDER_KRX, MARKET_KR)
            .expect("manifest")
            .is_empty()
    );
}

#[test]
fn active_entitlement_is_referenced_on_the_manifest_row() {
    let root = temp_root("ent-ref");
    let store = RawStore::new(&root);
    let recorded = RecordedBundle::open(GOOD_BUNDLE).expect("bundle opens");
    let provider = KrxProvider::synthetic(recorded);
    let service = active_entitlement_service();
    let req = request("2026-08-05T07:00:00Z");

    let outcome = ingest_bundle(&store, &provider, &req, Some(&service)).expect("ingest succeeds");
    assert_eq!(
        outcome.entry.entitlement_reference.as_deref(),
        Some("vault://krx-entitlements/ent_krx_2026_0001.pdf")
    );

    // Manifest read-back preserves the reference.
    let manifest = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest");
    assert_eq!(
        manifest[0].entitlement_reference,
        outcome.entry.entitlement_reference
    );
}

#[test]
fn typed_error_plumbing_from_provider_and_store() {
    // Every failure mode surfaces through the ingest pipeline as a typed error,
    // never a panic and never partial curated output (curation is Todo 10).
    let provider_err: IngestError = ProviderError::EndpointTimeout {
        kind: ResponseKind::Bars,
        timeout_secs: 30,
    }
    .into();
    assert!(matches!(
        provider_err,
        IngestError::Provider(ProviderError::EndpointTimeout { .. })
    ));

    let store_err: IngestError = StoreError::FileExists {
        path: "data/raw/...".to_owned(),
    }
    .into();
    assert!(matches!(
        store_err,
        IngestError::Store(StoreError::FileExists { .. })
    ));

    let unsupported: IngestError = ProviderError::UnsupportedKind(ResponseKind::Bars).into();
    assert!(matches!(
        unsupported,
        IngestError::Provider(ProviderError::UnsupportedKind(_))
    ));
}
