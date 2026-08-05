//! Todo 12 acceptance target: the fixed Korean ETF v1 universe manifest.
//!
//! `cargo test -p selector --test universe_manifest` parses
//! `configs/universes/kr-etf-core-v1.yaml`, resolves exactly 11 unique active
//! KRW ETFs plus the benchmark (`069500.KRX`) for the effective snapshot,
//! verifies no leverage/inverse flags, and writes an immutable
//! `universe_snapshot_id`; repeated builds hash identically.
//!
//! Publication must BLOCK (typed error naming the exact instrument + reason)
//! when an ID is inactive, unsupported, duplicated, or unlicensed — never
//! substitute a different product.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use auth::entitlement::{
    CalendarDate, ContractRef, DataProvider, DatasetId, DocumentHash, Entitlement,
    EntitlementService, EntitlementState, KrUse, UserId,
};
use domain::{
    AssetClass, ContentHash, DataState, InstrumentId, InstrumentStatus, Price, Quantity,
    TradingDate, Venue,
};
use market_data::{
    Instrument, InstrumentMaster, IssueCode, QualityIssue, QualityReport, Severity, seed_universe,
};
use selector::publish::{ProductKind, PublishedSnapshot, UniversePublisher};
use selector::universe::{
    Eligibility, SourceSnapshot, UniverseError, UniverseManifest, parse_manifest,
};
use tempfile::TempDir;

/// The fixed v1 universe (plan Todo 12): exact canonical IDs, manifest order.
const V1_SYMBOLS: [&str; 11] = [
    "069500", "102110", "229200", "143850", "133690", "195930", "192090", "148070", "114260",
    "153130", "132030",
];

/// Path of the committed v1 manifest relative to this crate.
fn manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/universes/kr-etf-core-v1.yaml")
}

fn load_v1_manifest() -> String {
    fs::read_to_string(manifest_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest_path().display()))
}

/// Inline YAML manifest builder for block-proof copies (never the real file).
#[allow(clippy::too_many_arguments)]
fn yaml(
    instruments: &[&str],
    benchmark: &str,
    unleveraged: bool,
    non_inverse: bool,
    asset_class: &str,
    source_version: &str,
) -> String {
    let list: Vec<String> = instruments
        .iter()
        .map(|s| format!("    - id: \"{s}.KRX\""))
        .collect();
    format!(
        "universe:\n  id: kr-etf-core-v1\n  base_currency: KRW\n  effective_from: \"2020-01-31\"\n  effective_until: null\n  benchmark: \"{benchmark}.KRX\"\n  eligibility:\n    unleveraged: {unleveraged}\n    non_inverse: {non_inverse}\n    asset_class: {asset_class}\n  instruments:\n{}\n  source_snapshot:\n    source: krx-reference-2019-v1\n    version: \"{source_version}\"\n    captured_at: \"2019-12-31\"\n",
        list.join("\n")
    )
}

fn v1_yaml() -> String {
    yaml(&V1_SYMBOLS, "069500", true, true, "etf", "1.0")
}

// ---------------------------------------------------------------------------
// Fixtures: masters, entitlements, publishers
// ---------------------------------------------------------------------------

fn seed_master() -> InstrumentMaster {
    seed_universe()
}

fn instrument(symbol: &str, name: &str) -> Instrument {
    Instrument {
        instrument_id: InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id"),
        name: name.to_owned(),
        asset_class: AssetClass::Etf,
        currency: domain::Currency::KRW,
        venue: Venue::Krx,
        listed_at: TradingDate::new(2019, 1, 1).expect("valid listing date"),
        delisted_at: None,
        price_increment: Price::parse("1").expect("valid tick"),
        size_increment: Quantity::parse("1").expect("valid size"),
        lot_size: Quantity::parse("100").expect("valid lot"),
        status: InstrumentStatus::Listed,
        reference_source: "krx-reference-2019-v1".to_owned(),
    }
}

/// A master with exactly the requested seed symbols, plus optional delisted /
/// suspended / extra-instrument variants of seed symbols.
fn master_with_mods(
    symbols: &[&str],
    delisted: &[&str],
    suspended: &[&str],
    extra: Vec<Instrument>,
) -> InstrumentMaster {
    let mut master = InstrumentMaster::new();
    for symbol in symbols {
        let mut record = instrument(symbol, &format!("SYNTHETIC-{symbol}"));
        if delisted.contains(symbol) {
            record.delisted_at = Some(TradingDate::new(2020, 1, 30).expect("valid delisting"));
        }
        if suspended.contains(symbol) {
            record.status = InstrumentStatus::Suspended;
        }
        master
            .register_instrument(record)
            .expect("test instrument registers");
    }
    for record in extra {
        master
            .register_instrument(record)
            .expect("test extra registers");
    }
    master
}

fn master_with(symbols: &[&str]) -> InstrumentMaster {
    master_with_mods(symbols, &[], &[], vec![])
}

fn hex(c: char) -> String {
    c.to_string().repeat(64)
}

fn entitlement(lifecycle: EntitlementState) -> Entitlement {
    Entitlement::builder()
        .id(auth::entitlement::EntitlementId::new("ent_krx_universe_v1"))
        .provider(DataProvider::Krx)
        .contract(ContractRef::new(
            DocumentHash::sha256(hex('0')),
            "vault://krx-entitlements/ent_krx_universe_v1.pdf",
        ))
        .lifecycle(lifecycle)
        .effective(
            CalendarDate::parse("2019-01-01").expect("valid window"),
            CalendarDate::parse("2030-12-31").expect("valid window"),
        )
        .covered_datasets([DatasetId::krx_eod_bars()])
        .covered_uses([KrUse::Dataset])
        .covered_users([UserId::new("universe_publisher")])
        .build()
}

fn active_entitlements() -> Vec<Entitlement> {
    vec![entitlement(EntitlementState::Active)]
}

fn publisher(master: InstrumentMaster, entitlements: Vec<Entitlement>) -> UniversePublisher {
    UniversePublisher::new(master, EntitlementService::new(entitlements))
}

fn id(symbol: &str) -> InstrumentId {
    InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id")
}

fn assert_block(
    result: Result<PublishedSnapshot, UniverseError>,
    contains: &[&str],
    variant: &str,
) {
    let err = match result {
        Ok(_) => panic!("publication unexpectedly succeeded"),
        Err(e) => e,
    };
    let text = err.to_string();
    for needle in contains {
        assert!(
            text.contains(needle),
            "block message {text:?} must contain {needle:?} (variant {variant})"
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance: parse -> resolve -> eligibility -> immutable snapshot id
// ---------------------------------------------------------------------------

#[test]
fn parses_fixed_v1_manifest_to_exactly_11_unique_krw_etfs_plus_benchmark() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    let snapshot = publisher(seed_master(), active_entitlements())
        .publish(&manifest)
        .expect("v1 universe publishes");

    // Exactly 11 unique instruments.
    assert_eq!(
        snapshot.instruments.len(),
        11,
        "exactly 11 unique instruments"
    );
    for symbol in V1_SYMBOLS {
        assert!(
            snapshot.instruments.contains(&id(symbol)),
            "snapshot contains {symbol}.KRX"
        );
    }
    // Benchmark is 069500.KRX and a member of the universe.
    assert_eq!(snapshot.benchmark, id("069500"));
    assert!(snapshot.instruments.contains(&snapshot.benchmark));
    // KRW base currency; every member is a KRW ETF.
    assert_eq!(snapshot.base_currency, domain::Currency::KRW);
    // Effective window.
    assert_eq!(
        snapshot.effective_from,
        TradingDate::new(2020, 1, 31).expect("valid date")
    );
    assert_eq!(snapshot.effective_until, None);
}

#[test]
fn eligibility_verifies_no_leverage_or_inverse_flags() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    let snapshot = publisher(seed_master(), active_entitlements())
        .publish(&manifest)
        .expect("v1 universe publishes");

    assert!(snapshot.eligibility.unleveraged, "no leverage");
    assert!(snapshot.eligibility.non_inverse, "no inverse");
    assert_eq!(snapshot.eligibility.asset_class, AssetClass::Etf);
    // The source snapshot metadata travels with the published snapshot.
    assert_eq!(snapshot.source_snapshot.source, "krx-reference-2019-v1");
    assert_eq!(snapshot.source_snapshot.version, "1.0");
}

#[test]
fn snapshot_hash_identical_across_two_builds() {
    let a = parse_manifest(&load_v1_manifest()).expect("manifest A parses");
    let b = parse_manifest(&load_v1_manifest()).expect("manifest B parses");

    // Two independent builds (fresh masters + services) hash identically.
    let sa = publisher(seed_master(), active_entitlements())
        .publish(&a)
        .expect("build A publishes");
    let sb = publisher(seed_master(), active_entitlements())
        .publish(&b)
        .expect("build B publishes");
    assert_eq!(sa.universe_snapshot_id, sb.universe_snapshot_id);
    assert!(sa.universe_snapshot_id.as_str().starts_with("sha256:"));

    // Two written snapshot files: identical filename + identical bytes.
    let dir_a = TempDir::new().expect("temp dir");
    let dir_b = TempDir::new().expect("temp dir");
    let path_a = sa.write_snapshot(dir_a.path()).expect("writes A");
    let path_b = sb.write_snapshot(dir_b.path()).expect("writes B");
    assert_eq!(
        path_a.file_name(),
        path_b.file_name(),
        "immutable snapshot naming: same hash -> same file"
    );
    assert_eq!(
        fs::read(path_a).expect("reads A"),
        fs::read(path_b).expect("reads B"),
        "immutable snapshot bytes: same hash -> same content"
    );
}

#[test]
fn changed_manifest_produces_new_snapshot_id() {
    let original = parse_manifest(&v1_yaml()).expect("original parses");
    let changed = parse_manifest(&yaml(&V1_SYMBOLS, "069500", true, true, "etf", "1.1"))
        .expect("changed parses");
    let h1 = original.canonical_hash().expect("hash 1");
    let h2 = changed.canonical_hash().expect("hash 2");
    assert_ne!(h1, h2, "metadata change must produce a new snapshot id");
}

#[test]
fn malformed_yaml_is_typed_error_not_panic() {
    // Truncated YAML: typed error, never a panic.
    let truncated = "universe:\n  id: kr-etf-core-v1\n  base_currency: [unclosed";
    match parse_manifest(truncated) {
        Err(UniverseError::MalformedManifest { .. }) => {}
        other => panic!("expected MalformedManifest, got {other:?}"),
    }
    // Unknown field: denied by the strict manifest contract.
    let unknown = "universe:\n  id: kr-etf-core-v1\n  base_currency: KRW\n  bogus_field: true\n  effective_from: \"2020-01-31\"\n  effective_until: null\n  benchmark: \"069500.KRX\"\n  eligibility:\n    unleveraged: true\n    non_inverse: true\n    asset_class: etf\n  instruments:\n    - id: \"069500.KRX\"\n  source_snapshot:\n    source: krx-reference-2019-v1\n    version: \"1.0\"\n    captured_at: \"2019-12-31\"\n";
    match parse_manifest(unknown) {
        Err(UniverseError::MalformedManifest { .. }) => {}
        other => panic!("expected MalformedManifest, got {other:?}"),
    }
}

#[test]
fn inverted_effective_window_is_typed_error() {
    let inverted = "universe:\n  id: kr-etf-core-v1\n  base_currency: KRW\n  effective_from: \"2020-01-31\"\n  effective_until: \"2020-01-01\"\n  benchmark: \"069500.KRX\"\n  eligibility:\n    unleveraged: true\n    non_inverse: true\n    asset_class: etf\n  instruments:\n    - id: \"069500.KRX\"\n  source_snapshot:\n    source: krx-reference-2019-v1\n    version: \"1.0\"\n    captured_at: \"2019-12-31\"\n";
    match parse_manifest(inverted) {
        Err(UniverseError::MalformedManifest { .. }) => {}
        other => panic!("expected MalformedManifest, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Publication blocks: duplicated
// ---------------------------------------------------------------------------

#[test]
fn duplicate_instrument_blocks_with_exact_id() {
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("069500"); // duplicate entry
    let manifest = parse_manifest(&yaml(&symbols, "069500", true, true, "etf", "1.0"))
        .expect("parses with duplicate");
    assert_block(
        publisher(seed_master(), active_entitlements()).publish(&manifest),
        &["069500.KRX", "duplicate"],
        "duplicate",
    );
}

// ---------------------------------------------------------------------------
// Publication blocks: inactive
// ---------------------------------------------------------------------------

#[test]
fn inactive_id_not_in_master_blocks_naming_instrument() {
    let missing: Vec<&str> = V1_SYMBOLS
        .iter()
        .copied()
        .filter(|s| *s != "102110")
        .collect();
    // The manifest (source of truth) still lists 102110.KRX; the master has
    // no record for it -> publication must name it, never substitute.
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    assert_block(
        publisher(master_with(&missing), active_entitlements()).publish(&manifest),
        &["102110.KRX", "not in instrument master"],
        "inactive-unknown",
    );
}

#[test]
fn inactive_delisted_id_blocks_naming_instrument() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    // 114260.KRX exists but is delisted before the effective date.
    let master = master_with_mods(&V1_SYMBOLS, &["114260"], &[], vec![]);
    assert_block(
        publisher(master, active_entitlements()).publish(&manifest),
        &["114260.KRX", "delisted"],
        "inactive-delisted",
    );
}

#[test]
fn inactive_suspended_status_blocks_naming_instrument() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    let master = master_with_mods(&V1_SYMBOLS, &[], &["192090"], vec![]);
    assert_block(
        publisher(master, active_entitlements()).publish(&manifest),
        &["192090.KRX", "suspended"],
        "inactive-suspended",
    );
}

#[test]
fn unknown_id_blocks_without_substitution() {
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("999999");
    let manifest = parse_manifest(&yaml(&symbols, "069500", true, true, "etf", "1.0"))
        .expect("parses with unknown id");
    // NEVER substitute a different product: the unknown id is named, no
    // other instrument takes its place.
    assert_block(
        publisher(seed_master(), active_entitlements()).publish(&manifest),
        &["999999.KRX", "not in instrument master"],
        "unknown-id",
    );
}

// ---------------------------------------------------------------------------
// Publication blocks: unsupported (asset class / leverage / inverse)
// ---------------------------------------------------------------------------

#[test]
fn unsupported_asset_class_blocks_naming_instrument() {
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("005930"); // an EQUITY (Samsung Electronics shape), not an ETF
    let manifest = parse_manifest(&yaml(&symbols, "069500", true, true, "etf", "1.0"))
        .expect("parses with equity");
    let mut extra = instrument("005930", "SYNTHETIC-SAMSUNG");
    extra.asset_class = AssetClass::Equity;
    let master = master_with_mods(&V1_SYMBOLS, &[], &[], vec![extra]);
    assert_block(
        publisher(master, active_entitlements()).publish(&manifest),
        &["005930.KRX", "asset class equity"],
        "unsupported-asset-class",
    );
}

#[test]
fn unsupported_leveraged_product_blocks_naming_instrument() {
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("122630"); // a leveraged ETF product
    let manifest = parse_manifest(&yaml(&symbols, "069500", true, true, "etf", "1.0"))
        .expect("parses with leveraged product");
    let master = master_with(&symbols);
    let kinds = BTreeMap::from([(id("122630"), ProductKind::Leveraged)]);
    let p = UniversePublisher::with_product_kinds(
        master,
        EntitlementService::new(active_entitlements()),
        kinds,
    );
    assert_block(
        p.publish(&manifest),
        &["122630.KRX", "leveraged"],
        "unsupported-leveraged",
    );
}

#[test]
fn unsupported_inverse_product_blocks_naming_instrument() {
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("233160"); // an inverse ETF product
    let manifest = parse_manifest(&yaml(&symbols, "069500", true, true, "etf", "1.0"))
        .expect("parses with inverse product");
    let master = master_with(&symbols);
    let kinds = BTreeMap::from([(id("233160"), ProductKind::Inverse)]);
    let p = UniversePublisher::with_product_kinds(
        master,
        EntitlementService::new(active_entitlements()),
        kinds,
    );
    assert_block(
        p.publish(&manifest),
        &["233160.KRX", "inverse"],
        "unsupported-inverse",
    );
}

#[test]
fn eligibility_flag_mutation_blocks_naming_exact_instrument() {
    // Mutate one eligibility flag in a COPY of the manifest: the fixed
    // universe is unleveraged/non-inverse spot ETFs only, so a universe that
    // permits leveraged products is unsupported and publication names the
    // exact instrument being gated.
    let manifest = parse_manifest(&yaml(&V1_SYMBOLS, "069500", false, true, "etf", "1.0"))
        .expect("parses with mutated eligibility");
    assert_block(
        publisher(seed_master(), active_entitlements()).publish(&manifest),
        &["069500.KRX", "unleveraged=false"],
        "eligibility-mutation",
    );
}

#[test]
fn benchmark_not_in_universe_blocks() {
    let without_benchmark: Vec<&str> = V1_SYMBOLS
        .iter()
        .copied()
        .filter(|s| *s != "069500")
        .collect();
    let manifest = parse_manifest(&yaml(
        &without_benchmark,
        "069500",
        true,
        true,
        "etf",
        "1.0",
    ))
    .expect("parses without benchmark member");
    assert_block(
        publisher(master_with(&without_benchmark), active_entitlements()).publish(&manifest),
        &["069500.KRX", "benchmark"],
        "benchmark-membership",
    );
}

#[test]
fn empty_instruments_blocks() {
    let manifest =
        parse_manifest(&yaml(&[], "069500", true, true, "etf", "1.0")).expect("parses empty list");
    match publisher(seed_master(), active_entitlements()).publish(&manifest) {
        Err(UniverseError::EmptyUniverse) => {}
        other => panic!("expected EmptyUniverse, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Publication blocks: unlicensed (Todo 5 entitlement gate)
// ---------------------------------------------------------------------------

#[test]
fn unlicensed_dataset_blocks_with_exact_reason() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    // EXPIRED entitlement: the KRX dataset use is not ACTIVE -> fail closed.
    let p = publisher(seed_master(), vec![entitlement(EntitlementState::Expired)]);
    assert_block(
        p.publish(&manifest),
        &[
            "krx_eod_bars",
            "DATA_ENTITLEMENT_REQUIRED",
            "EntitlementNotActive",
        ],
        "unlicensed-expired",
    );
}

#[test]
fn unlicensed_no_entitlement_record_blocks() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    // No entitlement record at all -> fail closed, never a silent success.
    assert_block(
        publisher(seed_master(), vec![]).publish(&manifest),
        &["krx_eod_bars", "DATA_ENTITLEMENT_REQUIRED"],
        "unlicensed-no-record",
    );
}

// ---------------------------------------------------------------------------
// Publication blocks: required dataset not READY (Todo 11 quality gate)
// ---------------------------------------------------------------------------

#[test]
fn required_data_not_ready_blocks_with_blocking_codes() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    let report = QualityReport {
        dataset_id: domain::DatasetId::parse("kr-etf-daily").expect("valid dataset id"),
        version: 1,
        state: DataState::Blocked,
        issues: vec![QualityIssue {
            code: IssueCode::MissingRequiredBar,
            severity: Severity::Blocking,
            instrument: Some(id("069500")),
            date: Some(TradingDate::new(2020, 2, 4).expect("valid date")),
            detail: "missing required bar on 2020-02-04".to_owned(),
        }],
        exclusions: vec![],
        content_hash: ContentHash::from_bytes(b"fixture-report"),
    };
    let p = publisher(seed_master(), active_entitlements()).with_required_data(report);
    assert_block(
        p.publish(&manifest),
        &["kr-etf-daily", "blocked", "MISSING_REQUIRED_BAR"],
        "required-data-not-ready",
    );
}

// ---------------------------------------------------------------------------
// Source snapshot metadata
// ---------------------------------------------------------------------------

#[test]
fn source_snapshot_metadata_round_trips_through_parse() {
    let manifest = parse_manifest(&load_v1_manifest()).expect("v1 manifest parses");
    assert_eq!(
        manifest.universe.source_snapshot,
        SourceSnapshot {
            source: "krx-reference-2019-v1".to_owned(),
            version: "1.0".to_owned(),
            captured_at: "2019-12-31".to_owned(),
        }
    );
    assert_eq!(
        manifest.universe.eligibility,
        Eligibility {
            unleveraged: true,
            non_inverse: true,
            asset_class: AssetClass::Etf,
        }
    );
    assert_eq!(manifest.universe.benchmark, id("069500"));
    let _: &UniverseManifest = &manifest;
}
