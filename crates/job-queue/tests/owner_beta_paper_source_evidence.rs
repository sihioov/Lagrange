use job_queue::owner_beta::paper::source_evidence::{self as wire, SESSION_LIMIT, SOURCE_LIMIT};
use serde_json::{Value, json};
const GOOD: &[u8] = include_bytes!("fixtures/owner_beta_paper_source_evidence/valid_chain.json");
const BAD: &[u8] = include_bytes!("fixtures/owner_beta_paper_source_evidence/rejections.json");
fn fixture() -> Value {
    wire::decode_canonical(GOOD, SESSION_LIMIT).unwrap()
}
fn rehash(events: &mut [Value]) {
    for index in 0..events.len() {
        events[index]["sequence"] = json!(index);
        events[index]["previous_sha256"] = if index == 0 {
            Value::Null
        } else {
            json!(wire::digest(&wire::canonical(&events[index - 1]).unwrap()))
        };
    }
}
#[test]
fn python_rust_shared_valid_chain() {
    let f = fixture();
    wire::validate_wire(&f).unwrap();
    wire::verify_chain(f["events"].as_array().unwrap(), Some("LINKED")).unwrap();
    assert_eq!(wire::canonical(&f).unwrap(), GOOD);
}
#[test]
fn shared_semantic_rejections() {
    let cases: Vec<Value> = serde_json::from_slice(BAD).unwrap();
    assert_eq!(cases.len(), 25);
    for case in cases {
        let mut f = fixture();
        let events = f["events"].as_array_mut().unwrap();
        let event = &mut events[case["event"].as_u64().unwrap() as usize];
        let path = case["path"].as_str().unwrap();
        if let Some(field) = event.pointer_mut(path) {
            *field = case["value"].clone();
        } else {
            let (parent, key) = path.rsplit_once('/').unwrap();
            event
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), case["value"].clone());
        }
        rehash(events);
        assert!(
            wire::verify_chain(events, Some("LINKED")).is_err(),
            "accepted {}",
            case["name"]
        );
    }
}
#[test]
fn crash_prefixes_and_deleted_events_never_complete() {
    let f = fixture();
    let events = f["events"].as_array().unwrap();
    for count in 1..events.len() {
        assert!(
            wire::verify_chain(&events[..count], Some("LINKED")).is_err(),
            "prefix {count}"
        );
    }
    let mut changed = events.clone();
    changed.remove(9);
    rehash(&mut changed);
    assert!(wire::verify_chain(&changed, Some("LINKED")).is_err());
}
#[test]
fn duplicate_unknown_noncanonical_and_resource_limits() {
    for bytes in [
        br#"{"a":1,"a":1}"#.as_slice(),
        br#"{ "a":1}"#,
        b"NaN",
        b"1.0",
    ] {
        assert!(wire::decode_canonical(bytes, SOURCE_LIMIT).is_err());
    }
    assert!(wire::decode_canonical(&vec![b' '; SOURCE_LIMIT + 1], SOURCE_LIMIT).is_err());
    let nested = format!("{}0{}", "[".repeat(17), "]".repeat(17));
    assert!(wire::decode_canonical(nested.as_bytes(), SOURCE_LIMIT).is_err());
    let mut f = fixture();
    f["events"][0]["success"] = json!(true);
    assert!(wire::validate_wire(&f).is_err());
}
#[test]
fn decoding_does_not_grant_filesystem_authority() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture();
    let slot = f["events"][0]["slot_id"].as_str().unwrap();
    let directory = temp.path().join(&slot[7..]);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("000000.json"),
        wire::canonical(&f["events"][0]).unwrap(),
    )
    .unwrap();
    assert!(wire::ProtectedSourceChain::open(temp.path(), slot).is_err());
}

#[test]
fn embedded_schema_matches_authoritative_document() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/contracts/owner-beta-paper-source-evidence-v1.schema.json");
    let expected: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let embedded: Value = serde_json::from_str(wire::SCHEMA_JSON).unwrap();
    assert_eq!(expected, embedded);
}

#[test]
fn production_python_orchestration_emits_shared_wire() {
    // Production Scheduler/Journal/relay/helper methods with test-only I/O.
    // SYNTHETIC: this does not confer protected FS, DB or timer authority.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("python3")
        .arg(root.join("scripts/ops/test_owner_beta_paper_evidence.py"))
        .arg("--emit-synthetic-chain")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .current_dir(root)
        .output()
        .expect("synthetic producer executable");
    assert!(output.status.success(), "synthetic producer failed");
    let value = wire::decode_canonical(&output.stdout, SESSION_LIMIT).unwrap();
    wire::validate_wire(&value).unwrap();
    let events = value["events"].as_array().unwrap();
    wire::verify_chain(events, Some("LINKED")).unwrap();
    assert_eq!(events.iter().filter(|e| e["kind"] == "RUNNING").count(), 1);
    assert_eq!(
        events.iter().filter(|e| e["kind"] == "RAW_SOURCE").count(),
        1
    );
    assert_eq!(
        events.iter().filter(|e| e["kind"] == "CONTAINER").count(),
        8
    );
}
