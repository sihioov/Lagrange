//! Owner-managed Korean equity universe V2 domain contracts.
//!
//! This module is deliberately separate from the deployed fixed-universe V1
//! contracts. It defines runtime policy, membership lifecycle, generation and
//! admission pins, typed failure codes, and the canonical active-universe
//! hash used by immutable signal snapshots.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::{CodeCommit, ContentHash, InstrumentId};

/// Recommended per-owner active-instrument limit for a newly provisioned
/// runtime policy. The persisted policy remains configurable without a schema
/// migration or application rebuild.
pub const RECOMMENDED_MAX_ACTIVE_INSTRUMENTS: u32 = 100;

/// Default number of recent observed sessions requested for a generation.
pub const DEFAULT_TARGET_OBSERVED_SESSIONS: u32 = 261;

/// Lowest admissible observed-session count for 20/60/120-session factors.
pub const MINIMUM_OBSERVED_SESSIONS: u32 = 121;

/// Validation errors for the owner-managed universe domain.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OwnerEquityUniverseError {
    /// Runtime policy values are incomplete or internally inconsistent.
    #[error("invalid owner equity universe policy")]
    InvalidPolicy,
    /// A lifecycle transition is not part of the explicit state graph.
    #[error("illegal owner equity membership transition: {from} -> {to}")]
    IllegalTransition {
        /// State before the attempted transition.
        from: OwnerEquityMembershipState,
        /// Requested next state.
        to: OwnerEquityMembershipState,
    },
    /// A persisted or wire state is not one of the eight explicit states.
    #[error("invalid owner equity membership state")]
    InvalidMembershipState,
    /// A generation number must be positive and may not overflow.
    #[error("invalid owner equity generation")]
    InvalidGeneration,
    /// Failure codes are bounded, uppercase machine identifiers, never prose.
    #[error("invalid owner equity failure code")]
    InvalidFailureCode,
    /// Active-ready universe members must be unique KRX six-digit instruments.
    #[error("invalid active-ready owner equity universe")]
    InvalidActiveReadyUniverse,
}

/// Runtime-configurable limits shared by the API and workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OwnerEquityUniversePolicy {
    max_active_instruments: u32,
    target_observed_sessions: u32,
    minimum_observed_sessions: u32,
}

impl OwnerEquityUniversePolicy {
    /// Builds a policy with the 261-session target and 121-observation minimum.
    pub fn with_max_active_instruments(
        max_active_instruments: u32,
    ) -> Result<Self, OwnerEquityUniverseError> {
        Self::new(
            max_active_instruments,
            DEFAULT_TARGET_OBSERVED_SESSIONS,
            MINIMUM_OBSERVED_SESSIONS,
        )
    }

    /// Validates an explicitly configured runtime policy.
    pub fn new(
        max_active_instruments: u32,
        target_observed_sessions: u32,
        minimum_observed_sessions: u32,
    ) -> Result<Self, OwnerEquityUniverseError> {
        if max_active_instruments == 0
            || minimum_observed_sessions < MINIMUM_OBSERVED_SESSIONS
            || target_observed_sessions < minimum_observed_sessions
        {
            return Err(OwnerEquityUniverseError::InvalidPolicy);
        }
        Ok(Self {
            max_active_instruments,
            target_observed_sessions,
            minimum_observed_sessions,
        })
    }

    /// Maximum number of non-disabled memberships for one owner.
    pub fn max_active_instruments(self) -> u32 {
        self.max_active_instruments
    }

    /// Requested recent observed-session coverage.
    pub fn target_observed_sessions(self) -> u32 {
        self.target_observed_sessions
    }

    /// Minimum coverage required before a generation may be admitted.
    pub fn minimum_observed_sessions(self) -> u32 {
        self.minimum_observed_sessions
    }
}

impl Default for OwnerEquityUniversePolicy {
    fn default() -> Self {
        Self::new(
            RECOMMENDED_MAX_ACTIVE_INSTRUMENTS,
            DEFAULT_TARGET_OBSERVED_SESSIONS,
            MINIMUM_OBSERVED_SESSIONS,
        )
        .expect("documented owner equity policy defaults are valid")
    }
}

#[derive(Deserialize)]
struct OwnerEquityUniversePolicyWire {
    max_active_instruments: u32,
    target_observed_sessions: u32,
    minimum_observed_sessions: u32,
}

impl<'de> Deserialize<'de> for OwnerEquityUniversePolicy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = OwnerEquityUniversePolicyWire::deserialize(deserializer)?;
        Self::new(
            wire.max_active_instruments,
            wire.target_observed_sessions,
            wire.minimum_observed_sessions,
        )
        .map_err(DeError::custom)
    }
}

/// Explicit membership lifecycle persisted by migration 0053.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnerEquityMembershipState {
    /// Durable owner request accepted for asynchronous processing.
    Requested,
    /// Canonical instrument identity is being validated.
    Validating,
    /// Read-only historical observations are being collected.
    Backfilling,
    /// An immutable per-instrument generation is being built and checked.
    Materializing,
    /// An admitted generation is eligible for signal snapshots.
    Ready,
    /// Valid coverage is below the configured minimum.
    InsufficientHistory,
    /// Processing stopped with a typed failure code.
    Failed,
    /// Soft-disabled by the owner; lineage remains durable.
    Disabled,
}

impl OwnerEquityMembershipState {
    /// Every legal persisted state.
    pub const ALL: &'static [Self] = &[
        Self::Requested,
        Self::Validating,
        Self::Backfilling,
        Self::Materializing,
        Self::Ready,
        Self::InsufficientHistory,
        Self::Failed,
        Self::Disabled,
    ];

    /// Database/wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "REQUESTED",
            Self::Validating => "VALIDATING",
            Self::Backfilling => "BACKFILLING",
            Self::Materializing => "MATERIALIZING",
            Self::Ready => "READY",
            Self::InsufficientHistory => "INSUFFICIENT_HISTORY",
            Self::Failed => "FAILED",
            Self::Disabled => "DISABLED",
        }
    }

    /// Whether `next` is an edge in the explicit lifecycle graph.
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Requested, Self::Validating | Self::Disabled)
                | (
                    Self::Validating,
                    Self::Backfilling | Self::Failed | Self::Disabled
                )
                | (
                    Self::Backfilling,
                    Self::Materializing | Self::InsufficientHistory | Self::Failed | Self::Disabled
                )
                | (
                    Self::Materializing,
                    Self::Ready | Self::InsufficientHistory | Self::Failed | Self::Disabled
                )
                | (Self::Ready, Self::Disabled)
                | (Self::InsufficientHistory, Self::Requested | Self::Disabled)
                | (Self::Failed, Self::Requested | Self::Disabled)
        )
    }

    /// Validates one state change.
    pub fn transition_to(self, next: Self) -> Result<Self, OwnerEquityUniverseError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(OwnerEquityUniverseError::IllegalTransition {
                from: self,
                to: next,
            })
        }
    }
}

impl fmt::Display for OwnerEquityMembershipState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for OwnerEquityMembershipState {
    type Err = OwnerEquityUniverseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == value)
            .ok_or(OwnerEquityUniverseError::InvalidMembershipState)
    }
}

/// Positive, monotonically allocated per-membership generation number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct OwnerEquityGeneration(u64);

impl OwnerEquityGeneration {
    /// Validates a positive generation.
    pub fn new(value: u64) -> Result<Self, OwnerEquityUniverseError> {
        if value == 0 {
            Err(OwnerEquityUniverseError::InvalidGeneration)
        } else {
            Ok(Self(value))
        }
    }

    /// Numeric generation value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The immediately following generation, failing on overflow.
    pub fn checked_next(self) -> Result<Self, OwnerEquityUniverseError> {
        self.0
            .checked_add(1)
            .ok_or(OwnerEquityUniverseError::InvalidGeneration)
            .and_then(Self::new)
    }
}

impl<'de> Deserialize<'de> for OwnerEquityGeneration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(deserializer)?).map_err(DeError::custom)
    }
}

/// Bounded machine-readable failure code. Provider prose is never accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerEquityFailureCode(String);

impl OwnerEquityFailureCode {
    /// Validates `[A-Z][A-Z0-9_]{0,63}`.
    pub fn parse(value: &str) -> Result<Self, OwnerEquityUniverseError> {
        let bytes = value.as_bytes();
        let valid = !bytes.is_empty()
            && bytes.len() <= 64
            && bytes[0].is_ascii_uppercase()
            && bytes
                .iter()
                .skip(1)
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_');
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(OwnerEquityUniverseError::InvalidFailureCode)
        }
    }

    /// Machine-readable code string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnerEquityFailureCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for OwnerEquityFailureCode {
    type Err = OwnerEquityUniverseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for OwnerEquityFailureCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for OwnerEquityFailureCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(DeError::custom)
    }
}

/// Explicit retry classification stored beside a typed failure code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetryDisposition {
    /// The owner may request another generation.
    Retryable,
    /// Retrying the same request is not allowed without a new contract/input.
    Terminal,
}

impl RetryDisposition {
    /// Database boolean representation.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Retryable)
    }
}

/// Exact immutable evidence pins recorded when a generation is admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEquityAdmissionPins {
    /// Immutable Raw batch/file manifest.
    pub raw_manifest_sha256: ContentHash,
    /// Immutable materialized artifact manifest.
    pub artifact_manifest_sha256: ContentHash,
    /// Exact entitlement contract hash.
    pub entitlement_sha256: ContentHash,
    /// Collector code revision.
    pub capture_code_commit: CodeCommit,
    /// Materializer/verifier code revision.
    pub materializer_code_commit: CodeCommit,
}

/// Exact hash of the sorted active-ready instrument identities.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OwnerEquityUniverseHash(ContentHash);

impl OwnerEquityUniverseHash {
    /// Computes SHA-256 over canonical instrument ids sorted ascending and
    /// joined by a single `\n`, with no trailing newline.
    pub fn from_active_ready<'a>(
        instruments: impl IntoIterator<Item = &'a InstrumentId>,
    ) -> Result<Self, OwnerEquityUniverseError> {
        let mut canonical = BTreeSet::new();
        let mut seen = 0usize;
        for instrument in instruments {
            if instrument.venue().as_str() != "KRX"
                || instrument.symbol().len() != 6
                || !instrument
                    .symbol()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
            {
                return Err(OwnerEquityUniverseError::InvalidActiveReadyUniverse);
            }
            seen += 1;
            canonical.insert(instrument.to_string());
        }
        if canonical.len() != seen {
            return Err(OwnerEquityUniverseError::InvalidActiveReadyUniverse);
        }
        let bytes = canonical.into_iter().collect::<Vec<_>>().join("\n");
        Ok(Self(ContentHash::from_bytes(bytes.as_bytes())))
    }

    /// Canonical `sha256:<hex>` representation.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the validated content hash.
    pub fn into_content_hash(self) -> ContentHash {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_and_validation_are_typed() {
        let policy = OwnerEquityUniversePolicy::default();
        assert_eq!(policy.max_active_instruments(), 100);
        assert_eq!(policy.target_observed_sessions(), 261);
        assert_eq!(policy.minimum_observed_sessions(), 121);
        assert!(OwnerEquityUniversePolicy::new(0, 261, 121).is_err());
        assert!(OwnerEquityUniversePolicy::new(100, 120, 121).is_err());
        assert!(OwnerEquityUniversePolicy::new(100, 261, 120).is_err());
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(
            serde_json::from_str::<OwnerEquityUniversePolicy>(&json).unwrap(),
            policy
        );
    }

    #[test]
    fn lifecycle_has_only_explicit_edges() {
        use OwnerEquityMembershipState as State;

        assert_eq!(State::ALL.len(), 8);
        assert!(State::Requested.can_transition_to(State::Validating));
        assert!(State::Materializing.can_transition_to(State::Ready));
        assert!(State::Failed.can_transition_to(State::Requested));
        assert!(State::Ready.can_transition_to(State::Disabled));
        assert!(!State::Requested.can_transition_to(State::Ready));
        assert!(!State::Disabled.can_transition_to(State::Requested));
        assert!(matches!(
            State::Requested.transition_to(State::Ready),
            Err(OwnerEquityUniverseError::IllegalTransition { .. })
        ));
        assert_eq!(serde_json::to_string(&State::Ready).unwrap(), "\"READY\"");
        assert_eq!(
            "INSUFFICIENT_HISTORY".parse(),
            Ok(State::InsufficientHistory)
        );
    }

    #[test]
    fn generations_and_failure_codes_reject_untyped_values() {
        let generation = OwnerEquityGeneration::new(1).unwrap();
        assert_eq!(generation.checked_next().unwrap().get(), 2);
        assert!(OwnerEquityGeneration::new(0).is_err());
        assert!(OwnerEquityFailureCode::parse("PROVIDER_THROTTLED").is_ok());
        assert!(OwnerEquityFailureCode::parse("provider said try later").is_err());
        assert!(RetryDisposition::Retryable.is_retryable());
        assert!(!RetryDisposition::Terminal.is_retryable());
    }

    #[test]
    fn universe_hash_is_order_independent_and_rejects_duplicates() {
        let first = InstrumentId::parse("005930.KRX").unwrap();
        let second = InstrumentId::parse("000660.KRX").unwrap();
        let left = OwnerEquityUniverseHash::from_active_ready([&first, &second]).unwrap();
        let right = OwnerEquityUniverseHash::from_active_ready([&second, &first]).unwrap();
        assert_eq!(left, right);
        assert!(left.as_str().starts_with("sha256:"));
        assert!(OwnerEquityUniverseHash::from_active_ready([&first, &first]).is_err());
        let etf = InstrumentId::parse("SPY.ARCA").unwrap();
        assert!(OwnerEquityUniverseHash::from_active_ready([&etf]).is_err());
    }
}
