//! Root-only, one-shot lifecycle consumer for the Owner-beta Paper operation ABI.
//!
//! The public surface is intentionally limited to parsing the fixed CLI verb and
//! consuming that verb. Paths, database identity, credentials, and checked wire
//! constructors remain private to this crate.
//!
//! Checked session/origin types are not public permits:
//! ```compile_fail
//! use job_queue::owner_beta::paper::evidence::CheckedEnableSession;
//! let _: CheckedEnableSession = serde_json::from_str("{}").unwrap();
//! ```
//! ```compile_fail
//! use job_queue::owner_beta::paper::evidence::CheckedArmOrigin;
//! fn copy_permit(value: &CheckedArmOrigin) -> CheckedArmOrigin { value.clone() }
//! ```

#[cfg(target_os = "linux")]
mod protected;
#[cfg(target_os = "linux")]
mod wire;

use serde::Serialize;

const RESULT_SCHEMA: &str = "owner-beta-paper-ops-result-v1";
const QUERY_RESULT_SCHEMA: &str = "owner-beta-paper-ops-query-result-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Configure,
    Inspect,
    Enable,
    Begin,
    Reconcile,
    Off,
    RestoreOff,
    Recover,
    Readback,
    ObserveReset,
}

impl Verb {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "configure" => Self::Configure,
            "inspect" => Self::Inspect,
            "enable" => Self::Enable,
            "begin" => Self::Begin,
            "reconcile" => Self::Reconcile,
            "off" => Self::Off,
            "restore-off" => Self::RestoreOff,
            "recover" => Self::Recover,
            "readback" => Self::Readback,
            "observe-reset" => Self::ObserveReset,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Configure => "configure",
            Self::Inspect => "inspect",
            Self::Enable => "enable",
            Self::Begin => "begin",
            Self::Reconcile => "reconcile",
            Self::Off => "off",
            Self::RestoreOff => "restore-off",
            Self::Recover => "recover",
            Self::Readback => "readback",
            Self::ObserveReset => "observe-reset",
        }
    }

    fn result_schema(self) -> &'static str {
        match self {
            Self::Readback | Self::ObserveReset => QUERY_RESULT_SCHEMA,
            _ => RESULT_SCHEMA,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Outcome {
    Ok,
    Replay,
    Blocked,
    CommitUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ErrorCode {
    None,
    WireInvalid,
    IdentityConflict,
    StaleEpoch,
    ClockInvalid,
    WriterUnknown,
    CommitUnknown,
    ReaderIncompatible,
    RecoveryAuthorityUnavailable,
    EvidenceUnavailable,
    InternalFailure,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultWire {
    schema: &'static str,
    verb: &'static str,
    outcome: Outcome,
    event_id: Option<String>,
    generation: Option<String>,
    control_epoch: Option<i64>,
    artifact_sha256: Option<String>,
    error_code: ErrorCode,
}

impl ResultWire {
    fn new(verb: Verb, outcome: Outcome, error_code: ErrorCode) -> Self {
        Self {
            schema: verb.result_schema(),
            verb: verb.as_str(),
            outcome,
            event_id: None,
            generation: None,
            control_epoch: None,
            artifact_sha256: None,
            error_code,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_else(|_| self.serialization_failure())
    }

    fn serialization_failure(&self) -> Vec<u8> {
        format!(
                "{{\"schema\":\"{}\",\"verb\":\"{}\",\"outcome\":\"BLOCKED\",\"event_id\":null,\"generation\":null,\"control_epoch\":null,\"artifact_sha256\":null,\"error_code\":\"INTERNAL_FAILURE\"}}",
                self.schema,
                self.verb
            )
            .into_bytes()
    }

    pub fn exit_code(&self) -> i32 {
        match self.outcome {
            Outcome::Ok | Outcome::Replay => 0,
            Outcome::Blocked => 3,
            Outcome::CommitUnknown => 4,
        }
    }
}

/// Consume one already-selected verb. No caller can provide a path, credential,
/// identity, SQL, or authority-bearing boolean to this API.
pub async fn run_native(verb: Verb) -> ResultWire {
    #[cfg(target_os = "linux")]
    {
        protected::run(verb).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        ResultWire::new(verb, Outcome::Blocked, ErrorCode::ReaderIncompatible)
    }
}

fn native_follow_up_block(verb: Verb) -> Option<ErrorCode> {
    match verb {
        // Restore/recovery still lack their consuming protected resource path.
        Verb::RestoreOff | Verb::Recover => Some(ErrorCode::EvidenceUnavailable),
        Verb::Configure
        | Verb::Inspect
        | Verb::Enable
        | Verb::Begin
        | Verb::Reconcile
        | Verb::Off
        | Verb::Readback
        | Verb::ObserveReset => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, Outcome, ResultWire, Verb, native_follow_up_block};
    use serde_json::Value;

    #[test]
    fn only_the_eight_fixed_verbs_parse() {
        let accepted = [
            "configure",
            "inspect",
            "enable",
            "begin",
            "reconcile",
            "off",
            "restore-off",
            "recover",
        ];
        for verb in accepted {
            assert!(Verb::parse(verb).is_some(), "{verb}");
        }
        for verb in ["", "help", "--help", "inspect/../x", "enable ", "ENABLE"] {
            assert!(Verb::parse(verb).is_none(), "{verb:?}");
        }
    }

    #[test]
    fn every_result_is_the_closed_public_shape_and_exit_contract() {
        let cases = [
            (Outcome::Ok, ErrorCode::None, 0),
            (Outcome::Replay, ErrorCode::None, 0),
            (Outcome::Blocked, ErrorCode::EvidenceUnavailable, 3),
            (Outcome::CommitUnknown, ErrorCode::CommitUnknown, 4),
        ];
        for (outcome, error_code, exit_code) in cases {
            let result = ResultWire::new(Verb::Begin, outcome, error_code);
            let value: Value = serde_json::from_slice(&result.encode()).unwrap();
            assert_eq!(value["schema"], "owner-beta-paper-ops-result-v1");
            assert_eq!(value["verb"], "begin");
            assert_eq!(value.as_object().unwrap().len(), 8);
            assert_eq!(result.exit_code(), exit_code);
        }
    }

    #[test]
    fn query_schema_and_serialization_failure_preserve_all_eight_old_verb_schemas() {
        for name in [
            "configure",
            "inspect",
            "enable",
            "begin",
            "reconcile",
            "off",
            "restore-off",
            "recover",
            "readback",
            "observe-reset",
        ] {
            let verb = Verb::parse(name).unwrap();
            for outcome in [
                Outcome::Ok,
                Outcome::Replay,
                Outcome::Blocked,
                Outcome::CommitUnknown,
            ] {
                let result = ResultWire::new(verb, outcome, ErrorCode::None);
                for bytes in [result.encode(), result.serialization_failure()] {
                    let value: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(value["verb"], name);
                    assert_eq!(
                        value["schema"],
                        if ["readback", "observe-reset"].contains(&name) {
                            "owner-beta-paper-ops-query-result-v1"
                        } else {
                            "owner-beta-paper-ops-result-v1"
                        }
                    );
                    assert_eq!(value.as_object().unwrap().len(), 8);
                }
            }
        }
    }

    #[test]
    fn missing_checker_and_receipt_chain_never_report_success() {
        for verb in [Verb::RestoreOff, Verb::Recover] {
            assert!(matches!(
                native_follow_up_block(verb),
                Some(ErrorCode::EvidenceUnavailable)
            ));
        }
        for verb in [
            Verb::Configure,
            Verb::Inspect,
            Verb::Enable,
            Verb::Begin,
            Verb::Reconcile,
            Verb::Off,
        ] {
            assert!(native_follow_up_block(verb).is_none());
        }
    }
}
