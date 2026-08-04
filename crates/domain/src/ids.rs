//! Branded identifiers.
//!
//! Every durable identity is a branded newtype with `Display`/`FromStr`/
//! `Serialize`/`Deserialize` — never a bare email, symbol, or UUID string.
//! Three families exist:
//!   - UUID-backed ids (`RunId`, `JobId`, `UserId`, ...): opaque, generated.
//!   - Slug ids (`StrategyId`, `FactorId`, `DatasetId`, ...): stable lowercase
//!     keys validated by an explicit character rule.
//!   - SemVer ids (`StrategyVersion`, `FactorVersion`): see `version.rs`.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::error::DomainError;

/// Defines an opaque UUID-backed branded ID.
macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            /// Wraps an existing UUID.
            pub fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            /// Generates a fresh random (v4) identifier.
            pub fn generate() -> Self {
                Self(Uuid::new_v4())
            }

            /// The underlying UUID.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self).map_err(|_| DomainError::InvalidId {
                    kind: stringify!($name).to_owned(),
                    value: s.to_owned(),
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                Self::from_str(&s).map_err(DeError::custom)
            }
        }
    };
}

/// Defines a slug-backed branded ID (lowercase letter or digit first, then
/// lowercase/digit/`_`/`-`/`.`, no trailing separator).
macro_rules! slug_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validates and wraps a slug string.
            pub fn parse(s: &str) -> Result<Self, DomainError> {
                let b = s.as_bytes();
                let valid = !b.is_empty()
                    && b.len() <= 96
                    && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
                    && b.iter().skip(1).all(|c| {
                        c.is_ascii_lowercase()
                            || c.is_ascii_digit()
                            || *c == b'_'
                            || *c == b'-'
                            || *c == b'.'
                    })
                    && !matches!(b[b.len() - 1], b'_' | b'-' | b'.');
                if valid {
                    Ok(Self(s.to_owned()))
                } else {
                    Err(DomainError::InvalidId {
                        kind: stringify!($name).to_owned(),
                        value: s.to_owned(),
                    })
                }
            }

            /// The slug string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                Self::parse(&s).map_err(DeError::custom)
            }
        }
    };
}

uuid_id! {
    /// Durable identity of a backtest/paper/live run (todo 20-21, 29).
    RunId
}
uuid_id! {
    /// Durable identity of a queued job (todo 19).
    JobId
}
uuid_id! {
    /// Durable identity of a single job attempt (todo 19).
    JobAttemptId
}
uuid_id! {
    /// Durable identity of an authenticated user (todo 22-23).
    UserId
}
uuid_id! {
    /// Durable identity of a generated artifact (todo 20, 28).
    ArtifactId
}
uuid_id! {
    /// Durable identity of a raw-ingestion batch (todo 8).
    BatchId
}
uuid_id! {
    /// Durable identity of a broker/engine order (todo 18, 36).
    OrderId
}
uuid_id! {
    /// Durable identity of an execution fill (todo 18, 36).
    FillId
}
uuid_id! {
    /// Durable identity of a position record (todo 18).
    PositionId
}
uuid_id! {
    /// Durable identity of a Paper account (todo 29).
    PaperAccountId
}
uuid_id! {
    /// Durable identity of a web session (todo 22).
    SessionId
}
uuid_id! {
    /// Correlation id propagated through API requests and logs (todo 24).
    CorrelationId
}
uuid_id! {
    /// Idempotency key for mutating API/queue operations (todo 19, 24).
    IdempotencyKey
}
uuid_id! {
    /// Durable identity of a strategy configuration (todo 17, 24).
    ConfigId
}
uuid_id! {
    /// Durable identity of an immutable universe snapshot (todo 12, 16).
    UniverseSnapshotId
}
uuid_id! {
    /// Durable identity of a provenance record (todo 6, 16, 20).
    ProvenanceId
}

slug_id! {
    /// Canonical strategy identifier (e.g. `dual_momentum`, todo 17).
    StrategyId
}
slug_id! {
    /// Canonical factor identifier (e.g. `momentum_12m`, todo 15).
    FactorId
}
slug_id! {
    /// Canonical dataset identifier (e.g. `kr-etf-daily`, todo 10-11).
    DatasetId
}
slug_id! {
    /// Canonical dataset version (e.g. `kr-etf-daily-20260804.1`, todo 10-11).
    DatasetVersionId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_ids_generate_and_parse() {
        let id = RunId::generate();
        let s = id.to_string();
        let back: RunId = s.parse().unwrap();
        assert_eq!(back, id);
        assert!(matches!("not-a-uuid".parse::<RunId>(), Err(DomainError::InvalidId { .. })));
        let json = serde_json::to_string(&id).unwrap();
        let again: RunId = serde_json::from_str(&json).unwrap();
        assert_eq!(again, id);
    }

    #[test]
    fn slug_ids_validate() {
        assert_eq!(StrategyId::parse("dual-momentum").unwrap().as_str(), "dual-momentum");
        assert_eq!(DatasetVersionId::parse("kr-etf-daily-20260804.1").unwrap().as_str(), "kr-etf-daily-20260804.1");
        assert!(matches!(
            StrategyId::parse("Dual Momentum!"),
            Err(DomainError::InvalidId { .. })
        ));
        assert!(matches!(FactorId::parse("-leading-dash"), Err(DomainError::InvalidId { .. })));
        assert!(matches!(FactorId::parse("trailing-"), Err(DomainError::InvalidId { .. })));
        assert!(matches!(FactorId::parse(""), Err(DomainError::InvalidId { .. })));
    }
}
