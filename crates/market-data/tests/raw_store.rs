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
use market_data::storage::{BatchSpec, RawStore, StoreError};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ls-task8-{tag}-{}", BatchId::generate()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec<'a>(b: BatchId, d: &'a TradingDate, ent: Option<&'a str>) -> BatchSpec<'a> {
    BatchSpec {
        provider: PROVIDER_KRX,
        market: MARKET_KR,
        date: d,
        batch_id: b,
        entitlement_reference: ent,
        mode: FetchMode::Synthetic,
    }
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

fn envelope(
    batch: BatchId,
    kind: ResponseKind,
    name: &str,
    bytes: &[u8],
    at: UtcTimestamp,
) -> RawEnvelope {
    RawEnvelope::new(
        batch,
        kind,
        name.to_owned(),
        bytes.to_vec(),
        at,
        meta(FetchMode::Synthetic),
    )
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
        .store_batch(&spec(b1, &d, None), &[e1])
        .expect("first delivery stores");

    let b2 = BatchId::generate();
    let e2 = envelope(b2, ResponseKind::Bars, "bars.json", &bytes, at);
    let entry2 = store
        .store_batch(&spec(b2, &d, None), &[e2])
        .expect("duplicate delivery stores as a new batch");

    // Two distinct batches, identical content hash.
    assert_ne!(
        entry1.batch_id, entry2.batch_id,
        "duplicate delivery must create a new batch"
    );
    assert_eq!(entry1.files[0].content_hash, entry2.files[0].content_hash);
    assert_eq!(
        entry1.files[0].content_hash,
        ContentHash::from_bytes(&bytes)
    );

    // First batch untouched: bytes on disk are byte-identical to the delivery.
    let back1 = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry1)
        .expect("first batch readable after duplicate delivery");
    assert_eq!(back1.len(), 1);
    assert_eq!(back1[0].file_name, "bars.json");
    assert_eq!(
        back1[0].bytes, bytes,
        "first batch bytes must never be modified"
    );

    // Exactly two batch dirs exist under the date partition.
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &d)
        .expect("batch listing");
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
        .store_batch(&spec(b1, &d, None), &[e1])
        .expect("first delivery");

    let m1 = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest readable");
    assert_eq!(m1.len(), 1);

    let b2 = BatchId::generate();
    let e2 = envelope(b2, ResponseKind::Calendar, "calendar.json", b"{}", at);
    store
        .store_batch(&spec(b2, &d, None), &[e2])
        .expect("second delivery");

    let m2 = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest readable");
    assert_eq!(m2.len(), 2, "manifest grows by one row per delivery");
    assert_eq!(
        m2[0], m1[0],
        "first manifest row must be byte-identical after append"
    );
    assert_ne!(m2[1].batch_id, m2[0].batch_id);

    // The manifest is a plain JSONL file with one line per batch: append-only, never rewritten.
    let raw =
        fs::read_to_string(store.manifest_path(PROVIDER_KRX, MARKET_KR)).expect("manifest file");
    assert_eq!(
        raw.lines().count(),
        2,
        "manifest file must hold exactly one line per batch"
    );
}

#[test]
fn manifest_entry_round_trips_through_json() {
    let root = temp_root("roundtrip");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T02:00:00Z");
    let d = date("2020-01-31");
    let b1 = BatchId::generate();
    let e1 = envelope(
        b1,
        ResponseKind::CorporateActions,
        "actions.json",
        b"[]",
        at,
    );
    let entry = store
        .store_batch(
            &spec(
                b1,
                &d,
                Some("vault://krx-entitlements/ent_krx_2026_0001.pdf"),
            ),
            &[e1],
        )
        .expect("stores");
    let json = serde_json::to_string(&entry).expect("serialize");
    let back: market_data::storage::ManifestEntry =
        serde_json::from_str(&json).expect("deserialize");
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
            .store_batch(&spec(b, &d, None), &[e])
            .expect_err("traversal file name must be rejected");
        assert!(
            matches!(err, StoreError::UnsafeFileName { .. }),
            "expected typed StoreError::UnsafeFileName for {bad:?}, got {err:?}"
        );
    }

    // Nothing was written for any rejected delivery.
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &d)
        .expect("batch listing");
    assert!(
        dirs.is_empty(),
        "no batch dirs may exist after rejected deliveries: {dirs:?}"
    );
    assert!(store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest")
        .is_empty());
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
        .store_batch(&spec(b1, &d, None), &dup)
        .expect_err("duplicate file name inside one batch must fail");
    assert!(
        matches!(err, StoreError::FileExists { .. }),
        "expected typed StoreError::FileExists, got {err:?}"
    );

    // No partial batch left behind: batch dir removed, manifest untouched.
    let dirs = store
        .batch_ids(PROVIDER_KRX, MARKET_KR, &d)
        .expect("batch listing");
    assert!(
        dirs.is_empty(),
        "failed batch must leave no batch dir: {dirs:?}"
    );
    assert!(store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect("manifest")
        .is_empty());
}

#[test]
fn read_detects_tampered_batch() {
    let root = temp_root("tamper");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T05:00:00Z");
    let d = date("2020-01-31");
    let b1 = BatchId::generate();
    let e1 = envelope(
        b1,
        ResponseKind::Reference,
        "reference.json",
        b"{\"ok\":true}",
        at,
    );
    let entry = store
        .store_batch(&spec(b1, &d, None), &[e1])
        .expect("stores");

    // Tamper with the stored bytes behind the store's back.
    let dir = store.batch_dir(PROVIDER_KRX, MARKET_KR, &d, &b1);
    let victim = dir.join("reference.json");
    fs::write(&victim, b"{\"ok\":false}").expect("tamper write");

    let err = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
        .expect_err("tampered content must be detected on read");
    match err {
        StoreError::ContentHashMismatch {
            path,
            recorded,
            actual,
        } => {
            assert!(path.ends_with("reference.json"));
            assert_eq!(recorded, entry.files[0].content_hash.to_string());
            assert_ne!(recorded, actual);
        }
        other => panic!("expected StoreError::ContentHashMismatch, got {other:?}"),
    }
}

#[test]
fn read_rejects_manifest_scope_mismatch_before_path_access() {
    let root = temp_root("scope-mismatch");
    let store = RawStore::new(&root);
    let at = now("2026-08-05T05:00:00Z");
    let d = date("2020-01-31");
    let batch = BatchId::generate();
    let mut entry = store
        .store_batch(
            &spec(batch, &d, None),
            &[envelope(
                batch,
                ResponseKind::Reference,
                "reference.json",
                b"{}",
                at,
            )],
        )
        .expect("stores");
    entry.provider = "other-provider".to_owned();

    let err = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
        .expect_err("scope mismatch must fail before reading a different scope");

    assert!(matches!(err, StoreError::ScopeMismatch { .. }));
}

#[test]
fn raw_store_rejects_unsafe_scope_and_deserialized_file_names() {
    let root = temp_root("unsafe-scope");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let batch = BatchId::generate();
    let unsafe_scope = store
        .store_batch(
            &BatchSpec {
                provider: "../outside",
                market: MARKET_KR,
                date: &d,
                batch_id: batch,
                entitlement_reference: None,
                mode: FetchMode::Synthetic,
            },
            &[],
        )
        .expect_err("unsafe provider scope must fail");
    assert!(matches!(unsafe_scope, StoreError::UnsafeScope { .. }));

    let unsafe_market = store
        .store_batch(
            &BatchSpec {
                provider: PROVIDER_KRX,
                market: "C:\\outside",
                date: &d,
                batch_id: BatchId::generate(),
                entitlement_reference: None,
                mode: FetchMode::Synthetic,
            },
            &[],
        )
        .expect_err("unsafe market scope must fail");
    assert!(matches!(unsafe_market, StoreError::UnsafeScope { .. }));

    let entry = store
        .store_batch(
            &spec(batch, &d, None),
            &[envelope(
                batch,
                ResponseKind::Reference,
                "reference.json",
                b"{}",
                now("2026-08-05T05:00:00Z"),
            )],
        )
        .expect("store safe batch");
    let mut untrusted = entry.clone();
    untrusted.files[0].file_name = "../outside.json".to_owned();
    let err = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &untrusted)
        .expect_err("deserialized traversal name must fail before join");
    assert!(matches!(err, StoreError::UnsafeFileName { .. }));
}

#[test]
fn read_manifest_rejects_embedded_scope_mismatch() {
    let root = temp_root("manifest-scope-mismatch");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let batch = BatchId::generate();
    let mut entry = store
        .store_batch(
            &spec(batch, &d, None),
            &[envelope(
                batch,
                ResponseKind::Reference,
                "reference.json",
                b"{}",
                now("2026-08-05T05:00:00Z"),
            )],
        )
        .expect("store safe batch");
    entry.market = "other-market".to_owned();
    let manifest_path = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string(&entry).unwrap()),
    )
    .expect("replace test manifest");

    let err = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect_err("embedded scope mismatch must fail");
    assert!(matches!(err, StoreError::ScopeMismatch { .. }));
}

#[test]
fn read_manifest_revalidates_deserialized_file_names() {
    let root = temp_root("manifest-file-name");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let batch = BatchId::generate();
    let mut entry = store
        .store_batch(
            &spec(batch, &d, None),
            &[envelope(
                batch,
                ResponseKind::Reference,
                "reference.json",
                b"{}",
                now("2026-08-05T05:00:00Z"),
            )],
        )
        .expect("store safe batch");
    entry.files[0].file_name = r"..\outside.json".to_owned();
    let manifest_path = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string(&entry).unwrap()),
    )
    .expect("replace test manifest");

    let err = store
        .read_manifest(PROVIDER_KRX, MARKET_KR)
        .expect_err("unsafe manifest file name must fail");
    assert!(matches!(err, StoreError::UnsafeFileName { .. }));
}

#[cfg(windows)]
#[test]
fn read_rejects_symlinked_file_that_escapes_the_batch_directory() {
    use std::os::windows::fs::symlink_file;

    let root = temp_root("symlink-escape");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let batch = BatchId::generate();
    let entry = store
        .store_batch(
            &spec(batch, &d, None),
            &[envelope(
                batch,
                ResponseKind::Reference,
                "reference.json",
                b"{}",
                now("2026-08-05T05:00:00Z"),
            )],
        )
        .expect("store safe batch");
    let link = store
        .batch_dir(PROVIDER_KRX, MARKET_KR, &d, &batch)
        .join("reference.json");
    let outside = root.join("outside.json");
    fs::write(&outside, b"{}").expect("write outside target");
    fs::remove_file(&link).expect("remove stored file");
    if let Err(error) = symlink_file(&outside, &link) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            return;
        }
        panic!("create symlink: {error}");
    }

    let err = store
        .read_batch_bytes(PROVIDER_KRX, MARKET_KR, &entry)
        .expect_err("symlink escape must fail");
    assert!(matches!(err, StoreError::UnsafePath { .. }));
}

#[test]
fn documented_raw_layout_holds() {
    // Store root is the `data/` dir; raw zone is data/raw/... per System Design 7.1.
    let base = temp_root("layout");
    let root = base.join("data");
    fs::create_dir_all(&root).expect("create data root");
    let store = RawStore::new(&root);
    let d = date("2020-01-31");
    let b1 = BatchId::generate();

    let dir = store.batch_dir(PROVIDER_KRX, MARKET_KR, &d, &b1);
    let dir_str = dir.to_string_lossy().replace('\\', "/");
    assert!(
        dir_str.contains("data/raw/"),
        "raw zone root missing: {dir_str}"
    );
    assert!(
        dir_str.contains("provider=krx"),
        "provider partition missing: {dir_str}"
    );
    assert!(
        dir_str.contains("market=kr"),
        "market partition missing: {dir_str}"
    );
    assert!(
        dir_str.contains("date=2020-01-31"),
        "date partition missing: {dir_str}"
    );
    assert!(
        dir_str.contains(&format!("batch={b1}")),
        "batch dir missing: {dir_str}"
    );

    let mpath = store.manifest_path(PROVIDER_KRX, MARKET_KR);
    let mstr = mpath.to_string_lossy().replace('\\', "/");
    assert!(
        mstr.contains("manifests"),
        "manifest must live under data/raw/manifests: {mstr}"
    );
    assert!(
        mstr.ends_with("manifest.jsonl"),
        "manifest must be JSONL: {mstr}"
    );
    assert!(Path::new(&root).is_dir(), "temp root must exist");
}
