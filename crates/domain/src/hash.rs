//! Content hashes and code-commit references used by provenance contracts.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A content-addressable hash in `sha256:<64 lowercase hex>` form.
///
/// Raw/Curated/Artifact manifests reference batches by this hash so that
/// immutability is verifiable without storing payloads inline.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(String);

impl ContentHash {
    /// Computes the SHA-256 content hash of `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        Self(format!("sha256:{hex}"))
    }

    /// Validates a `sha256:<64 hex>` string, normalizing to lowercase.
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let rest = s
            .strip_prefix("sha256:")
            .ok_or_else(|| DomainError::InvalidContentHash {
                value: s.to_owned(),
            })?;
        if rest.len() == 64 && rest.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Self(format!("sha256:{}", rest.to_ascii_lowercase())))
        } else {
            Err(DomainError::InvalidContentHash {
                value: s.to_owned(),
            })
        }
    }

    /// The full `sha256:<hex>` string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The hash algorithm prefix (`sha256`).
    pub fn algorithm(&self) -> &'static str {
        "sha256"
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ContentHash {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

/// A git commit reference (7-64 lowercase hex digits), normalized to lowercase.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CodeCommit(String);

impl CodeCommit {
    /// Validates and wraps a git commit reference.
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let b = s.as_bytes();
        if (7..=64).contains(&b.len()) && b.iter().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(s.to_ascii_lowercase()))
        } else {
            Err(DomainError::InvalidCodeCommit {
                value: s.to_owned(),
            })
        }
    }

    /// The normalized hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CodeCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CodeCommit {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for CodeCommit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CodeCommit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_round_trip_and_rejection() {
        let h = ContentHash::from_bytes(b"hello world");
        assert_eq!(h.algorithm(), "sha256");
        assert_eq!(h.as_str().len(), 7 + 64);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), h);

        assert!(matches!(
            ContentHash::parse("md5:abc"),
            Err(DomainError::InvalidContentHash { .. })
        ));
        assert!(matches!(
            ContentHash::parse("sha256:short"),
            Err(DomainError::InvalidContentHash { .. })
        ));
        // uppercase hex is accepted and normalized to lowercase
        let upper = format!("sha256:{}", "A".repeat(64));
        let lower = format!("sha256:{}", "a".repeat(64));
        assert_eq!(ContentHash::parse(&upper).unwrap(), ContentHash::parse(&lower).unwrap());
    }

    #[test]
    fn code_commit_validation() {
        assert_eq!(CodeCommit::parse("ABCDEF123").unwrap().as_str(), "abcdef123");
        assert!(matches!(
            CodeCommit::parse("xyz"),
            Err(DomainError::InvalidCodeCommit { .. })
        ));
        assert!(matches!(
            CodeCommit::parse("abc"),
            Err(DomainError::InvalidCodeCommit { .. })
        ));
    }
}
