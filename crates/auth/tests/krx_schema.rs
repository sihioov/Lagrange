//! Smoke tests for the redacted KRX schema/example and the owner-approved KIS
//! personal-use entitlement record.
//!
//! Full schema validation of instances is performed by the Python `jsonschema`
//! gate in the Todo 5 evidence (see `.omo/evidence/task-5-lagrange-station-implementation.json`);
//! these tests keep the committed config pinned to the contract (lifecycle values,
//! covered sets, and the no-contract-contents redaction rule).

use std::fs;

use sha2::{Digest, Sha256};

const SCHEMA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../configs/data-rights/krx.schema.json"
);
const EXAMPLE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../configs/data-rights/krx.entitlement.example.json"
);
const KIS_ENTITLEMENT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../configs/data-rights/kis.entitlement.json"
);
const KIS_ATTESTATION_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/decisions/0005-kis-personal-use-entitlement.md"
);

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("missing config file {path}: {e}"))
}

#[test]
fn schema_requires_redacted_entitlement_metadata() {
    let raw = read(SCHEMA_PATH);
    assert!(raw.starts_with('{'), "schema must be a JSON object");
    for needle in [
        "\"provider\"",
        "\"krx\"",
        "\"lifecycle\"",
        "\"PENDING\"",
        "\"ACTIVE\"",
        "\"EXPIRED\"",
        "\"REVOKED\"",
        "\"contract_document\"",
        "\"document_hash\"",
        "\"document_reference\"",
        "\"covered_datasets\"",
        "\"covered_uses\"",
        "\"covered_users\"",
        "\"effective_from\"",
        "\"effective_until\"",
        "\"additionalProperties\": false",
        "\"enum\": [\"PENDING\", \"ACTIVE\", \"EXPIRED\", \"REVOKED\"]",
    ] {
        assert!(raw.contains(needle), "schema missing {needle}");
    }
    // Redaction: contract contents are never part of the schema.
    assert!(
        !raw.contains("\"contents\""),
        "schema must not admit contract contents"
    );
    assert!(
        !raw.contains("\"terms\""),
        "schema must not admit contract terms"
    );
}

#[test]
fn example_metadata_is_valid_shape_and_redacted() {
    let raw = read(EXAMPLE_PATH);
    for needle in [
        "\"provider\": \"krx\"",
        "\"lifecycle\": \"ACTIVE\"",
        "\"covered_users\": [\"usr_a\", \"usr_b\", \"usr_c\", \"usr_d\", \"usr_e\"]",
        "\"document_reference\": \"vault://krx-entitlements/ent_krx_2026_0001.pdf\"",
    ] {
        assert!(raw.contains(needle), "example missing {needle}");
    }
    assert!(
        !raw.contains("\"contents\""),
        "example must not carry contract contents"
    );

    // The committed placeholder hash must still satisfy the 64-hex pattern.
    let hex_marker = "\"hex\": \"";
    let start = raw.find(hex_marker).expect("hash hex present") + hex_marker.len();
    let hex = &raw[start..start + 64];
    assert!(
        hex.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "example hash must be 64 lowercase hex chars"
    );
}

#[test]
fn covered_uses_match_the_rust_registry() {
    let raw = read(SCHEMA_PATH);
    for use_kind in [
        "dataset",
        "factor",
        "recommendation",
        "backtest",
        "report",
        "benchmark",
        "paper_view",
        "payload",
        "download",
    ] {
        assert!(
            raw.contains(&format!("\"{use_kind}\"")),
            "schema enum missing {use_kind}"
        );
    }
    // Dev uses are deliberately absent: Owner-only development is not an entitlement.
    for dev_use in [
        "dev_ingest",
        "dev_curate",
        "dev_factor",
        "dev_backtest",
        "dev_report",
    ] {
        assert!(
            !raw.contains(dev_use),
            "dev use {dev_use} must not be an entitlement use"
        );
    }
}

#[test]
fn kis_entitlement_is_active_owner_only_and_binds_the_attestation() {
    let raw = read(KIS_ENTITLEMENT_PATH);
    let metadata: serde_json::Value =
        serde_json::from_str(&raw).expect("KIS entitlement must be valid JSON");

    assert_eq!(metadata["provider"], "kis");
    assert_eq!(metadata["lifecycle"], "ACTIVE");
    assert_eq!(metadata["covered_users"], serde_json::json!(["usr_owner"]));
    assert_eq!(
        metadata["contract_document"]["document_reference"],
        "repo://docs/decisions/0005-kis-personal-use-entitlement.md"
    );

    let attestation = fs::read(KIS_ATTESTATION_PATH)
        .unwrap_or_else(|e| panic!("missing KIS owner attestation: {e}"));
    let attestation_hash = hex::encode(Sha256::digest(&attestation));
    assert_eq!(
        metadata["contract_document"]["document_hash"]["hex"], attestation_hash,
        "KIS entitlement hash must bind the committed owner attestation"
    );
}
