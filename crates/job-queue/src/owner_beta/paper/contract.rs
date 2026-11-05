//! Untrusted Paper witness inputs and machine-readable eligibility results.
//!
//! Decoding the installed witness binding validates its closed source schema;
//! it does not confer source, rights, price, or activation authority.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::source_evidence as wire;

pub const PAPER_CONTRACT_LIMIT: usize = 65_536;
pub const PAPER_SESSION_LIMIT: usize = 262_144;
pub const PAPER_ELIGIBILITY_LIMIT: usize = 1_048_576;
pub const PAPER_MAX_DEPTH: usize = 16;
pub const ELIGIBILITY_TTL_SECONDS: i64 = 180;

const WITNESS_BINDING_SCHEMA: &str = "owner-beta-paper-witness-binding-v1";
const _: () = assert!(PAPER_MAX_DEPTH == wire::MAX_DEPTH);

/// A canonical, schema-checked binding selector. Its fields remain untrusted
/// until an evidence reader independently opens and verifies every source.
#[derive(Clone, Debug, PartialEq)]
pub struct UntrustedPaperContractV1 {
    binding: Value,
}

impl UntrustedPaperContractV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, PaperContractDecodeError> {
        if bytes.len() > PAPER_CONTRACT_LIMIT {
            return Err(PaperContractDecodeError::LimitExceeded);
        }
        let binding = wire::decode_canonical(bytes, PAPER_CONTRACT_LIMIT)
            .map_err(|_| PaperContractDecodeError::InvalidWire)?;
        if binding["schema"] != WITNESS_BINDING_SCHEMA {
            return Err(PaperContractDecodeError::WrongSchema);
        }
        wire::validate_wire(&binding).map_err(|_| PaperContractDecodeError::InvalidWire)?;
        Ok(Self { binding })
    }

    /// The value is a selector only. Callers cannot turn it into a verified
    /// input by retaining or reserializing this value.
    pub fn as_untrusted_value(&self) -> &Value {
        &self.binding
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperContractDecodeError {
    #[error("PAPER_CONTRACT_LIMIT_EXCEEDED")]
    LimitExceeded,
    #[error("PAPER_CONTRACT_INVALID_WIRE")]
    InvalidWire,
    #[error("PAPER_CONTRACT_WRONG_SCHEMA")]
    WrongSchema,
}

/// Stable denial vocabulary for Paper source eligibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaperEligibilityReasonCode {
    CalendarUnproven,
    CohortChanged,
    DuplicateIdentity,
    EligibilityExpired,
    EvidenceMissing,
    EvidenceTampered,
    FutureSession,
    InputNotAdmitted,
    InputPinMismatch,
    ManualIntervention,
    OriginUnproven,
    OwnerScopeMismatch,
    PriceNotAvailable,
    ReaderIncompatible,
    ReplayDetected,
    RightsUnavailable,
    SessionNotConsecutive,
    SourceRevoked,
    StrategyBindingMismatch,
}

impl PaperEligibilityReasonCode {
    pub const fn code(self) -> &'static str {
        match self {
            Self::CalendarUnproven => "CALENDAR_UNPROVEN",
            Self::CohortChanged => "COHORT_CHANGED",
            Self::DuplicateIdentity => "DUPLICATE_IDENTITY",
            Self::EligibilityExpired => "ELIGIBILITY_EXPIRED",
            Self::EvidenceMissing => "EVIDENCE_MISSING",
            Self::EvidenceTampered => "EVIDENCE_TAMPERED",
            Self::FutureSession => "FUTURE_SESSION",
            Self::InputNotAdmitted => "INPUT_NOT_ADMITTED",
            Self::InputPinMismatch => "INPUT_PIN_MISMATCH",
            Self::ManualIntervention => "MANUAL_INTERVENTION",
            Self::OriginUnproven => "ORIGIN_UNPROVEN",
            Self::OwnerScopeMismatch => "OWNER_SCOPE_MISMATCH",
            Self::PriceNotAvailable => "PRICE_NOT_AVAILABLE",
            Self::ReaderIncompatible => "READER_INCOMPATIBLE",
            Self::ReplayDetected => "REPLAY_DETECTED",
            Self::RightsUnavailable => "RIGHTS_UNAVAILABLE",
            Self::SessionNotConsecutive => "SESSION_NOT_CONSECUTIVE",
            Self::SourceRevoked => "SOURCE_REVOKED",
            Self::StrategyBindingMismatch => "STRATEGY_BINDING_MISMATCH",
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.code()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperEligibilityDecision {
    Eligible,
    Ineligible,
}

/// Read-only result data. This is not an action permit and is intentionally
/// neither serializable nor deserializable as an authority-bearing receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperEligibilityReport {
    decision: PaperEligibilityDecision,
    reasons: Vec<PaperEligibilityReasonCode>,
    sessions: Vec<String>,
    evaluated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl PaperEligibilityReport {
    pub(crate) fn new(
        decision: PaperEligibilityDecision,
        mut reasons: Vec<PaperEligibilityReasonCode>,
        sessions: Vec<String>,
        evaluated_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        reasons.sort_unstable();
        reasons.dedup();
        let decision = if reasons.is_empty() {
            decision
        } else {
            PaperEligibilityDecision::Ineligible
        };
        Self {
            decision,
            reasons,
            sessions,
            evaluated_at,
            expires_at,
        }
    }

    pub fn decision(&self) -> PaperEligibilityDecision {
        self.decision
    }

    pub fn reason_codes(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.reasons.iter().map(|reason| reason.code())
    }

    pub fn sessions(&self) -> &[String] {
        &self.sessions
    }

    pub fn evaluated_at(&self) -> DateTime<Utc> {
        self.evaluated_at
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Emit the read-only eligibility result in the closed canonical wire
    /// format. The bytes contain no admission or activation capability.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, PaperEligibilityEncodeError> {
        let value = json!({
            "decision": match self.decision {
                PaperEligibilityDecision::Eligible => "ELIGIBLE",
                PaperEligibilityDecision::Ineligible => "INELIGIBLE",
            },
            "evaluated_at": self.evaluated_at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            "expires_at": self.expires_at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            "reason_codes": self.reasons.iter().map(|reason| reason.code()).collect::<Vec<_>>(),
            "schema": "owner-beta-paper-eligibility-v1",
            "sessions": self.sessions,
        });
        let bytes =
            wire::canonical(&value).map_err(|_| PaperEligibilityEncodeError::InvalidWire)?;
        if bytes.len() > PAPER_ELIGIBILITY_LIMIT {
            return Err(PaperEligibilityEncodeError::LimitExceeded);
        }
        wire::decode_canonical(&bytes, PAPER_ELIGIBILITY_LIMIT)
            .map_err(|_| PaperEligibilityEncodeError::InvalidWire)?;
        Ok(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperEligibilityEncodeError {
    #[error("PAPER_ELIGIBILITY_LIMIT_EXCEEDED")]
    LimitExceeded,
    #[error("PAPER_ELIGIBILITY_INVALID_WIRE")]
    InvalidWire,
}

pub(crate) fn denied(
    reason: PaperEligibilityReasonCode,
    evaluated_at: DateTime<Utc>,
) -> PaperEligibilityReport {
    PaperEligibilityReport::new(
        PaperEligibilityDecision::Ineligible,
        vec![reason],
        Vec::new(),
        evaluated_at,
        evaluated_at,
    )
}
