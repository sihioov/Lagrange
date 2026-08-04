//! Semantic version types for strategies and factors.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A semantic version `major.minor.patch` with an optional pre-release tag
/// (e.g. `1.2.0`, `1.2.0-beta.1`). Pre-release tags are stored verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl SemVer {
    /// Parses a SemVer string, rejecting malformed input (including leading
    /// zeros in numeric segments).
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let invalid = || DomainError::InvalidVersion {
            value: s.to_owned(),
        };
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_owned())),
            _ => (s, None),
        };
        let mut parts = core.split('.');
        let major = parse_segment(parts.next(), &invalid)?;
        let minor = parse_segment(parts.next(), &invalid)?;
        let patch = parse_segment(parts.next(), &invalid)?;
        if parts.next().is_some() {
            return Err(invalid());
        }
        if let Some(pre) = &pre {
            let ok = pre
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
            if !ok || pre.starts_with('.') || pre.ends_with('.') {
                return Err(invalid());
            }
        }
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// The `major` component.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// The `minor` component.
    pub fn minor(&self) -> u64 {
        self.minor
    }

    /// The `patch` component.
    pub fn patch(&self) -> u64 {
        self.patch
    }

    /// The optional pre-release tag (without the leading `-`).
    pub fn pre_release(&self) -> Option<&str> {
        self.pre.as_deref()
    }
}

fn parse_segment(
    part: Option<&str>,
    invalid: &dyn Fn() -> DomainError,
) -> Result<u64, DomainError> {
    let part = part.ok_or_else(invalid)?;
    if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
        return Err(invalid());
    }
    part.parse::<u64>().map_err(|_| invalid())
}

impl fmt::Display for SemVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl FromStr for SemVer {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for SemVer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SemVer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

/// Defines a branded newtype over [`SemVer`].
macro_rules! semver_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(SemVer);

        impl $name {
            /// Parses a semantic version string.
            pub fn parse(s: &str) -> Result<Self, DomainError> {
                Ok(Self(SemVer::parse(s)?))
            }

            /// The inner semantic version.
            pub fn inner(&self) -> &SemVer {
                &self.0
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
                Self::parse(s)
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
                Self::parse(&s).map_err(DeError::custom)
            }
        }
    };
}

semver_id! {
    /// Immutable version of a strategy package (todo 17 promotion registry).
    StrategyVersion
}
semver_id! {
    /// Immutable version of a factor definition (todo 15).
    FactorVersion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let v = StrategyVersion::parse("1.2.0").unwrap();
        assert_eq!(v.to_string(), "1.2.0");
        let v2 = StrategyVersion::parse("1.2.0-beta.1").unwrap();
        assert_eq!(v2.to_string(), "1.2.0-beta.1");
        let json = serde_json::to_string(&v2).unwrap();
        assert_eq!(json, "\"1.2.0-beta.1\"");
        assert_eq!(serde_json::from_str::<StrategyVersion>(&json).unwrap(), v2);
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["1.2", "1.2.3.4", "01.2.3", "1.2.x", "1.2.3-", "", "a.b.c"] {
            assert!(
                matches!(SemVer::parse(bad), Err(DomainError::InvalidVersion { .. })),
                "expected rejection for {bad:?}"
            );
        }
    }
}
