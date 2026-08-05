//! Red-phase contract tests for the **immutable Raw storage zone** (Todo 8).
//!
//! Proven here:
//! - identical provider bytes ingested twice => TWO batches, SAME content hash,
//!   the first batch never modified;
//! - the raw manifest is append-only (JSONL, entries never rewritten);
//! - provider file names are validated: path traversal is rejected as a typed
//!   error with no partial batch;
//! - within-batch filename collisions fail with no partial batch left behind;
//! - reads verify the stored content hash (tamper detection);
//! - the documented `data/raw/provider=krx/market=kr/date=...` layout holds.
//!
//! Manual QA channel: `cargo test -p market-data --test raw_store -- --nocapture`

use std::fs;
use std::path::{Path, PathBuf};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use market_data::contract::{
    FetchMode, RawEnvelope, RequestMetadata, ResponseKind, MARKET_KR, PROVIDER_KRX,
};
use market_data::storage::{RawStore, StoreError};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ls-task8-{tag}-{}", BatchId::generate()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn date(s: &str) -> TradingDate {
    TradingDate::parse(s).expect("valid date")
}

fn now(s: &str) -> UtcTimestamp {
    UtcTimestamp::parse_rfc3339(s).expect("valid timestamp")
}

fn meta(mode: FetchMode) -> RequestMetadata {
    RequestMetadata {
        endpoint: "krx.eod.bars.v1".to_owned(),
        query: vec![("market".to_owned(), "KR".to_owned())],
        headers: vec![("X-Data-License".to_owned(), "redacted".to_owned())],
        mode,
    }
}

fn envelope(batch: BatchId, kind: ResponseKind, name: &str, bytes: &[u8], at: UtcTimestamp) -> RawEnvelope {
    RawEnvelope::new(batch, kind, name.to_owned(), bytes.to_vec(), at, meta(FetchMode::Synthetic))
}

#[test]
fn identical_bytes_twice_two_batches_same_hash_first_untouched() {
    let root = temp_root("dup");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T00:00:00Z");
    let d = date("2020-01-31");
    let bytes = br#"{"dataset":"synthetic","bars":[]}"#.to_vec();

    let b1 = BatchId::generate();
    let e1 = envelope(b1, ResponseKind::Bars, "bars.json", &bytes, at);
    let entry1 = store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b1, None, FetchMode::Synthetic, &[e1])
        .expect("first delivery stores");

    let b2 = BatchId::generate();
    let e2 = envelope(b2, ResponseKind::Bars, "bars.json", &bytes, at);
    let entry2 = store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b2, None, FetchMode::Synthetic, &[e2])
        .expect("duplicate delivery stores as a new batch");

    // Two distinct batches, identical content hash.
    assert_ne!(entry1.batch_id, entry2.batch_id, "duplicate delivery must create a new batch");
    assert_eq!(entry1.files[0].content_hash, entry2.files[0].content_hash);
    assert_eq!(entry1.files[0].content_hash, ContentHash::from_bytes(&bytes));

    // First batch untouched: bytes on disk are byte-identical to the delivery.
    let back1 = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry1)
        .expect("first batch readable after duplicate delivery");
    assert_eq!(back1.len(), 1);
    assert_eq!(back1[0].file_name, "bars.json");
    assert_eq!(back1[0].bytes, bytes, "first batch bytes must never be modified");

    // Exactly two batch dirs exist under the date partition.
    let dirs = store.batch_ids(PROVIDER_KRX, MARKET_KR, &d).expect("batch listing");
    assert_eq!(dirs.len(), 2, "exactly two batches expected, got {dirs:?}");
    assert!(dirs.contains(&b1));
    assert!(dirs.contains(&b2));
}

#[test]
fn manifest_is_append_only() {
    let root = temp_root("manifest");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T01:00:00Z");
    let d = date("2020-01-31");

    let b1 = BatchId::generate();
    let e1 = envelope(b1, ResponseKind::Calendar, "calendar.json", b"{}", at);
    store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b1, None, FetchMode::Synthetic, &[e1])
        .expect("first delivery");

    let m1 = store.read_manifest(PROVIDER_KRX, MARKET_KR).expect("manifest readable");
    assert_eq!(m1.len(), 1);

    let b2 = BatchId::generate();
    let e2 = envelope(b2, ResponseKind::Calendar, "calendar.json", b"{}", at);
    store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b2, None, FetchMode::Synthetic, &[e2])
        .expect("second delivery");

    let m2 = store.read_manifest(PROVIDER_KRX, MARKET_KR).expect("manifest readable");
    assert_eq!(m2.len(), 2, "manifest grows by one row per delivery");
    assert_eq!(m2[0], m1[0], "first manifest row must be byte-identical after append");
    assert_ne!(m2[1].batch_id, m2[0].batch_id);

    // The manifest is a plain JSONL file with one line per batch: append-only, never rewritten.
    let raw = fs::read_to_string(store.manifest_path(PROVIDER_KRX, MARKET_KR)).expect("manifest file");
    assert_eq!(raw.lines().count(), 2, "manifest file must hold exactly one line per batch");
}

#[test]
fn manifest_entry_round_trips_through_json() {
    let root = temp_root("roundtrip");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T02:00:00Z");
    let d = date("2020-01-31");
    let b1 = BatchId::generate();
    let e1 = envelope(b1, ResponseKind::CorporateActions, "actions.json", b"[]", at);
    let entry = store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b1, Some("vault://krx-entitlements/ent_krx_2026_0001.pdf"), FetchMode::Synthetic, &[e1])
        .expect("stores");
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: market_data::storage::ManifestEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, entry);
    assert!(json.contains("entitlement_reference"));
    assert!(json.contains("retrieved_at"));
    assert!(json.contains("content_hash"));
    assert!(json.contains("batch_id"));
}

#[test]
fn path_traversal_filenames_rejected_with_no_partial_batch() {
    let root = temp_root("traversal");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T03:00:00Z");
    let d = date("2020-01-31");

    let evil_names = [
        r"..\..\evil.json", // Windows traversal (QA scenario from the plan)
        "../evil.json",     // POSIX traversal
        "a/b.json",         // separator inside name
        "a\\b.json",
        "/etc/passwd",
        "C:\\evil.json", // drive-absolute
        "..",
        ".",
        "",
    ];
    for bad in evil_names {
        let b = BatchId::generate();
        let e = envelope(b, ResponseKind::Bars, bad, b"{}", at);
        let err = store
            .store_batch(PROVIDER_KRX, MARKET_KR, &d, b, None, FetchMode::Synthetic, &[e])
            .expect_err("traversal file name must be rejected");
        assert!(
            matches!(err, StoreError::UnsafeFileName { .. }),
            "expected typed StoreError::UnsafeFileName for {bad:?}, got {err:?}"
        );
    }

    // Nothing was written for any rejected delivery.
    let dirs = store.batch_ids(PROVIDER_KRX, MARKET_KR, &d).expect("batch listing");
    assert!(dirs.is_empty(), "no batch dirs may exist after rejected deliveries: {dirs:?}");
    assert!(store.read_manifest(PROVIDER_KRX, MARKET_KR).expect("manifest").is_empty());
}

#[test]
fn within_batch_collision_fails_without_partial_batch() {
    let root = temp_root("collision");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T04:00:00Z");
    let d = date("2020-01-31");
    let b1 = BatchId::generate();

    // Two envelopes with the SAME file name inside one batch: second write must fail.
    let dup = [
        envelope(b1, ResponseKind::Bars, "bars.json", b"first", at),
        envelope(b1, ResponseKind::Bars, "bars.json", b"second", at),
    ];
    let err = store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b1, None, FetchMode::Synthetic, &dup)
        .expect_err("duplicate file name inside one batch must fail");
    assert!(
        matches!(err, StoreError::FileExists { .. }),
        "expected typed StoreError::FileExists, got {err:?}"
    );

    // No partial batch left behind: batch dir removed, manifest untouched.
    let dirs = store.batch_ids(PROVIDER_KRX, MARKET_KR, &d).expect("batch listing");
    assert!(dirs.is_empty(), "failed batch must leave no batch dir: {dirs:?}");
    assert!(store.read_manifest(PROVIDER_KRX, MARKET_KR).expect("manifest").is_empty());
}

#[test]
fn read_detects_tampered_batch() {
    let root = temp_root("tamper");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T05:00:00Z");
    let d = date("2020-01-31");
    let b1 = BatchId::generate();
    let e1 = envelope(b1, ResponseKind::Reference, "reference.json", b"{\"ok\":true}", at);
    let entry = store
        .store_batch(PROVIDER_KRX, MARKET_KR, &d, b1, None, FetchMode::Synthetic, &[e1])
        .expect("stores");

    // Tamper with the stored bytes behind the store's back.
    let dir = store.batch_dir(PROVIDER_KRX, MARKET_KR, &d, &b1);
    let victim = dir.join("reference.json");
    fs::write(&victim, b"{\"ok\":false}").expect("tamper write");

    let err = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
        .expect_err("tampered content must be detected on read");
    match err {
        StoreError::Io { context, detail } => {
            assert!(
                context.to_lowercase().contains("hash") || detail.to_lowercase().contains("hash"),
                "tamper error must mention the hash check, got {context} / {detail}"
            );
        }
        other => panic!("expected StoreError::Io for tampered batch, got {other:?}"),
    }
}

#[test]
fn documented_raw_layout_holds() {
    let root = temp_root("layout");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let b1 = BatchId::generate();

    let dir = store.batch_dir(PROVIDER_KRX, MARKET_KR, &d, &b1);
    let dir_str = dir.to_string_lossy().replace('\\', "/");
    assert!(dir_str.contains("data/raw/"), "raw zone root missing: {dir_str}");
    assert!(dir_str.contains("provider=krx"), "provider partition missing: {dir_str}");
    assert!(dir_str.contains("market=kr"), "market partition missing: {dir_str}");
    assert!(dir_str.contains("date=2020-01-31"), "date partition missing: {dir_str}");
    assert!(dir_str.contains(&format!("batch={b1}")), "batch dir missing: {dir_str}");

    let mpath = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    let mstr = mpath.to_string_lossy().replace('\\', "/");
    assert!(mstr.contains("manifests"), "manifest must live under data/raw/manifests: {mstr}");
    assert!(mstr.ends_with("manifest.jsonl"), "manifest must be JSONL: {mstr}");
    assert!(Path::new(&root).is_dir(), "temp root must exist");
}
