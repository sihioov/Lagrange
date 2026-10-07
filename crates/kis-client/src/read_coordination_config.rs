//! Explicit runtime selection for process-shared KIS read coordination.
//!
//! Production has one fixed shared-state location and owner. Tests exercise
//! the parser as a pure function and inject private roots through
//! `ReadCoordinationConfig`; no environment variable can redirect production
//! coordination state.

use std::path::PathBuf;

use crate::read_coordination::{PRODUCTION_EXPECTED_UID, ReadCoordinationConfig};

pub const COORDINATION_MODE_ENV: &str = "KIS_READ_COORDINATION_MODE";
pub const CREDENTIAL_GENERATION_ENV: &str = "KIS_READ_CREDENTIAL_GENERATION";
pub const INTRADAY_QUOTES_MODE_ENV: &str = "OWNER_INTRADAY_QUOTES_MODE";
pub const PRODUCTION_STATE_ROOT: &str = "/run/lagrange/kis-read-coordination";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadCoordinationMode {
    Legacy,
    SharedRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayQuotesMode {
    Disabled,
    OwnerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntradayQuoteTransport {
    Rest,
    MarketWs,
}

impl IntradayQuoteTransport {
    pub fn from_optional_str(value: Option<&str>) -> Result<Self, ReadCoordinationConfigError> {
        match value {
            None | Some("rest") => Ok(Self::Rest),
            Some("market_ws") => Ok(Self::MarketWs),
            Some(_) => Err(ReadCoordinationConfigError::InvalidIntradayTransport),
        }
    }

    pub fn for_mode(
        mode: IntradayQuotesMode,
        value: Option<&str>,
    ) -> Result<Self, ReadCoordinationConfigError> {
        let transport = Self::from_optional_str(value)?;
        if mode == IntradayQuotesMode::OwnerOnly && transport != Self::MarketWs {
            return Err(ReadCoordinationConfigError::InvalidIntradayTransport);
        }
        Ok(transport)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadCoordinationConfigError {
    #[error("KIS read coordination mode is invalid")]
    InvalidCoordinationMode,
    #[error("owner intraday quotes mode is invalid")]
    InvalidIntradayMode,
    #[error("owner intraday quote transport is invalid")]
    InvalidIntradayTransport,
    #[error("owner-only intraday quotes require shared KIS read coordination")]
    IntradayRequiresShared,
    #[error("KIS read credential generation must be a canonical positive decimal")]
    InvalidCredentialGeneration,
    #[error("KIS read coordination environment is not valid UTF-8")]
    NonUnicodeEnvironment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductionReadCoordination {
    mode: ReadCoordinationMode,
    intraday_mode: IntradayQuotesMode,
    generation: Option<u64>,
}

impl ProductionReadCoordination {
    pub fn from_values(
        coordination_mode: Option<&str>,
        intraday_mode: Option<&str>,
        credential_generation: Option<&str>,
    ) -> Result<Self, ReadCoordinationConfigError> {
        let mode = match coordination_mode {
            None => ReadCoordinationMode::Legacy,
            Some("legacy") => ReadCoordinationMode::Legacy,
            Some("shared_required") => ReadCoordinationMode::SharedRequired,
            Some(_) => return Err(ReadCoordinationConfigError::InvalidCoordinationMode),
        };
        let intraday_mode = match intraday_mode {
            None | Some("off") => IntradayQuotesMode::Disabled,
            Some("owner_only") => IntradayQuotesMode::OwnerOnly,
            Some(_) => return Err(ReadCoordinationConfigError::InvalidIntradayMode),
        };
        if intraday_mode == IntradayQuotesMode::OwnerOnly
            && mode != ReadCoordinationMode::SharedRequired
        {
            return Err(ReadCoordinationConfigError::IntradayRequiresShared);
        }
        let generation = if mode == ReadCoordinationMode::SharedRequired {
            let value = credential_generation
                .ok_or(ReadCoordinationConfigError::InvalidCredentialGeneration)?;
            let parsed = value
                .parse::<u64>()
                .ok()
                .filter(|parsed| *parsed > 0 && parsed.to_string() == value)
                .ok_or(ReadCoordinationConfigError::InvalidCredentialGeneration)?;
            Some(parsed)
        } else {
            None
        };
        Ok(Self {
            mode,
            intraday_mode,
            generation,
        })
    }

    pub fn from_env() -> Result<Self, ReadCoordinationConfigError> {
        fn optional(name: &str) -> Result<Option<String>, ReadCoordinationConfigError> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(std::env::VarError::NotUnicode(_)) => {
                    Err(ReadCoordinationConfigError::NonUnicodeEnvironment)
                }
            }
        }

        let coordination = optional(COORDINATION_MODE_ENV)?;
        let intraday = optional(INTRADAY_QUOTES_MODE_ENV)?;
        let generation = optional(CREDENTIAL_GENERATION_ENV)?;
        Self::from_values(
            coordination.as_deref(),
            intraday.as_deref(),
            generation.as_deref(),
        )
    }

    pub const fn mode(self) -> ReadCoordinationMode {
        self.mode
    }

    pub const fn intraday_mode(self) -> IntradayQuotesMode {
        self.intraday_mode
    }

    pub fn generation(self) -> Option<u64> {
        self.generation
    }

    pub fn production_config(self) -> Option<ReadCoordinationConfig> {
        (self.mode == ReadCoordinationMode::SharedRequired).then(|| {
            ReadCoordinationConfig::new(
                PathBuf::from(PRODUCTION_STATE_ROOT),
                PRODUCTION_EXPECTED_UID,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_matrix_is_explicit_default_off_and_legacy_compatible() {
        let default = ProductionReadCoordination::from_values(None, None, None).unwrap();
        assert_eq!(default.mode(), ReadCoordinationMode::Legacy);
        assert_eq!(default.intraday_mode(), IntradayQuotesMode::Disabled);
        assert_eq!(default.generation(), None);

        let legacy = ProductionReadCoordination::from_values(
            Some("legacy"),
            Some("off"),
            Some("ignored-for-legacy"),
        )
        .unwrap();
        assert_eq!(legacy.mode(), ReadCoordinationMode::Legacy);

        let shared = ProductionReadCoordination::from_values(
            Some("shared_required"),
            Some("owner_only"),
            Some("17"),
        )
        .unwrap();
        assert_eq!(shared.mode(), ReadCoordinationMode::SharedRequired);
        assert_eq!(shared.intraday_mode(), IntradayQuotesMode::OwnerOnly);
        assert_eq!(shared.generation(), Some(17));
        let config = shared.production_config().unwrap();
        assert_eq!(config.state_root().to_str(), Some(PRODUCTION_STATE_ROOT));
    }

    #[test]
    fn intraday_quote_transport_parser_accepts_only_exact_values() {
        assert_eq!(
            IntradayQuoteTransport::from_optional_str(None),
            Ok(IntradayQuoteTransport::Rest)
        );
        assert_eq!(
            IntradayQuoteTransport::from_optional_str(Some("rest")),
            Ok(IntradayQuoteTransport::Rest)
        );
        assert_eq!(
            IntradayQuoteTransport::from_optional_str(Some("market_ws")),
            Ok(IntradayQuoteTransport::MarketWs)
        );

        for value in [
            "",
            " ",
            "rest ",
            " REST",
            "REST",
            "Market_ws",
            "market_WS",
            "unknown",
        ] {
            assert_eq!(
                IntradayQuoteTransport::from_optional_str(Some(value)),
                Err(ReadCoordinationConfigError::InvalidIntradayTransport),
                "{value:?} must be rejected without normalization"
            );
        }
    }

    #[test]
    fn intraday_quote_transport_is_required_only_when_intraday_is_enabled() {
        for value in [None, Some("rest"), Some("market_ws")] {
            assert!(IntradayQuoteTransport::for_mode(IntradayQuotesMode::Disabled, value).is_ok());
        }
        assert_eq!(
            IntradayQuoteTransport::for_mode(IntradayQuotesMode::Disabled, None),
            Ok(IntradayQuoteTransport::Rest)
        );
        assert_eq!(
            IntradayQuoteTransport::for_mode(IntradayQuotesMode::OwnerOnly, Some("market_ws")),
            Ok(IntradayQuoteTransport::MarketWs)
        );
        for value in [None, Some("rest")] {
            assert_eq!(
                IntradayQuoteTransport::for_mode(IntradayQuotesMode::OwnerOnly, value),
                Err(ReadCoordinationConfigError::InvalidIntradayTransport)
            );
        }
        for value in [Some(""), Some(" market_ws"), Some("REST"), Some("unknown")] {
            assert_eq!(
                IntradayQuoteTransport::for_mode(IntradayQuotesMode::Disabled, value),
                Err(ReadCoordinationConfigError::InvalidIntradayTransport)
            );
            assert_eq!(
                IntradayQuoteTransport::for_mode(IntradayQuotesMode::OwnerOnly, value),
                Err(ReadCoordinationConfigError::InvalidIntradayTransport)
            );
        }
    }

    #[test]
    fn empty_unknown_and_owner_only_legacy_values_fail_closed() {
        for value in ["", "shared", "SHARED_REQUIRED", " legacy"] {
            assert_eq!(
                ProductionReadCoordination::from_values(Some(value), None, None),
                Err(ReadCoordinationConfigError::InvalidCoordinationMode)
            );
        }
        for value in ["", "disabled", "OWNER_ONLY"] {
            assert_eq!(
                ProductionReadCoordination::from_values(None, Some(value), None),
                Err(ReadCoordinationConfigError::InvalidIntradayMode)
            );
        }
        assert_eq!(
            ProductionReadCoordination::from_values(Some("legacy"), Some("owner_only"), None),
            Err(ReadCoordinationConfigError::IntradayRequiresShared)
        );
    }

    #[test]
    fn shared_generation_is_mandatory_positive_and_canonical() {
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("01"),
            Some("+1"),
            Some(" 1"),
        ] {
            assert_eq!(
                ProductionReadCoordination::from_values(
                    Some("shared_required"),
                    Some("off"),
                    value
                ),
                Err(ReadCoordinationConfigError::InvalidCredentialGeneration)
            );
        }
        assert_eq!(
            ProductionReadCoordination::from_values(
                Some("shared_required"),
                None,
                Some("18446744073709551615")
            )
            .unwrap()
            .generation(),
            Some(u64::MAX)
        );
    }
}
