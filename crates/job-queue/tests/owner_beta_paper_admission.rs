use job_queue::owner_beta::paper::{
    contract::{PaperContractDecodeError, UntrustedPaperContractV1},
    input::{PaperInputError, UntrustedPaperSessionReadbackV1},
};
use serde_json::Value;

const VALID_BINDING: &[u8] = include_bytes!("fixtures/owner_beta_paper/valid_binding.json");

#[test]
fn paper_binding_decoder_accepts_canonical_closed_fixture_as_untrusted() {
    let decoded = UntrustedPaperContractV1::decode(VALID_BINDING).expect("fixture is valid");
    let value = decoded.as_untrusted_value();
    assert_eq!(value["schema"], "owner-beta-paper-witness-binding-v1");
    assert_eq!(
        value["cohort"]["owner_user_id"],
        "7b1f9ad8-29f1-4bf5-9ce6-980e8db0905b"
    );
}

#[test]
fn paper_binding_decoder_rejects_unknown_keys_and_wrong_schema() {
    let mut value: Value = serde_json::from_slice(VALID_BINDING).expect("fixture JSON");
    value["unexpected_authority"] = Value::Bool(true);
    let bytes = serde_json::to_vec(&value).expect("canonical JSON value");
    assert_eq!(
        UntrustedPaperContractV1::decode(&bytes),
        Err(PaperContractDecodeError::InvalidWire)
    );

    let mut value: Value = serde_json::from_slice(VALID_BINDING).expect("fixture JSON");
    value["schema"] = Value::String("owner-beta-paper-v1".to_owned());
    let bytes = serde_json::to_vec(&value).expect("canonical JSON value");
    assert_eq!(
        UntrustedPaperContractV1::decode(&bytes),
        Err(PaperContractDecodeError::WrongSchema)
    );
}

#[test]
fn paper_binding_decoder_enforces_canonical_bytes_depth_and_size() {
    let mut with_whitespace = VALID_BINDING.to_vec();
    with_whitespace.push(b' ');
    assert_eq!(
        UntrustedPaperContractV1::decode(&with_whitespace),
        Err(PaperContractDecodeError::InvalidWire)
    );

    assert_eq!(
        UntrustedPaperContractV1::decode(&vec![b' '; 65_537]),
        Err(PaperContractDecodeError::LimitExceeded)
    );

    let mut nested = Value::Null;
    for _ in 0..17 {
        nested = Value::Array(vec![nested]);
    }
    let too_deep = serde_json::to_vec(&nested).expect("nested JSON");
    assert_eq!(
        UntrustedPaperContractV1::decode(&too_deep),
        Err(PaperContractDecodeError::InvalidWire)
    );
}

#[test]
fn session_readback_decoder_rejects_caller_supplied_success_claims() {
    let claim = br#"{"schema":"owner-beta-paper-db-v1","terminal":"SUCCEEDED"}"#;
    assert_eq!(
        UntrustedPaperSessionReadbackV1::decode(claim),
        Err(PaperInputError::InvalidWire)
    );

    assert_eq!(
        UntrustedPaperSessionReadbackV1::decode(&vec![b' '; 262_145]),
        Err(PaperInputError::LimitExceeded)
    );
}
