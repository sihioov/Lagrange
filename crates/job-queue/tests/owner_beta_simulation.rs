use domain::ContentHash;
use job_queue::owner_beta::simulation::{
    EngineIdentity, SimulationRunError, SimulationRunIdentity, SimulationRunRequest,
    run_approved_historical_price_only_simulation,
};
use market_data::approve_historical_price_only_artifact_for_pin_values;
use uuid::Uuid;

const APPROVED_V3_REGISTRY: &[u8] = include_bytes!(
    "../../../configs/evidence/kis-historical-price-only-v3-approved-artifacts.json"
);

fn registry_hash() -> ContentHash {
    ContentHash::from_bytes(APPROVED_V3_REGISTRY)
}

fn approved_pins() -> [ContentHash; 5] {
    let registry: serde_json::Value =
        serde_json::from_slice(APPROVED_V3_REGISTRY).expect("checked-in v3 registry");
    let record = &registry["approved_artifacts"][0];
    [
        "candidate_content_sha256",
        "artifact_manifest_sha256",
        "stage5_manifest_sha256",
        "action_manifest_sha256",
    ]
    .into_iter()
    .map(|name| {
        ContentHash::parse(record[name].as_str().expect("pin string")).expect("checked-in pin")
    })
    .chain(std::iter::once(registry_hash()))
    .collect::<Vec<_>>()
    .try_into()
    .expect("five pins")
}

fn synthetic_identity() -> SimulationRunIdentity {
    let revision = "a".repeat(40);
    let binary = format!("sha256:{}", "b".repeat(64));
    let engine = EngineIdentity::new(
        &revision,
        &binary,
        "rustc-test",
        "x86_64-unknown-linux-gnu",
        "test",
    )
    .expect("synthetic declarative identity");
    SimulationRunIdentity::new(engine)
}

#[test]
fn approval_rejects_each_tampered_v3_pin_without_opening_an_artifact() {
    let pins = approved_pins();
    let root = tempfile::tempdir().expect("private test root");
    let artifact_root = root.path().join("must-not-be-opened");
    let wrong = ContentHash::parse(&format!("sha256:{}", "0".repeat(64))).expect("wrong pin");

    for changed_index in 0..pins.len() {
        let mut changed = pins.clone();
        changed[changed_index] = wrong.clone();
        assert!(
            approve_historical_price_only_artifact_for_pin_values(
                &artifact_root,
                &changed[0],
                &changed[1],
                &changed[2],
                &changed[3],
                &changed[4],
            )
            .is_err()
        );
    }
}

#[test]
fn unavailable_approved_artifact_fails_before_signal_or_result_creation() {
    let root = tempfile::tempdir().expect("private test root");
    let request = SimulationRunRequest::new(Uuid::nil(), Uuid::nil());
    let error = run_approved_historical_price_only_simulation(
        &root.path().join("no-approved-artifact"),
        &request,
        &synthetic_identity(),
    )
    .expect_err("approval must fail closed");
    assert_eq!(error, SimulationRunError::ApprovalRejected);
}
