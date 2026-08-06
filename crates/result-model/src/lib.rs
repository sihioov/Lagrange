//! `result-model` — Lagrange Station common result model.
//!
//! The typed warning/error envelope shared by every downstream todo: the API
//! (design §12.1 `code`, `message`, `correlation_id`, `details`), the backtest
//! worker normalizer (design §6.10 `warnings`), reports, and Paper/Live.
//! Plus the normalized [`BacktestResult`] common model (design §6.10) and the
//! T3 database manifest writer (plan Todo 20).

use serde::{Deserialize, Serialize};

use domain::DomainError;
use domain::ids::CorrelationId;

pub mod backtest;
pub mod manifest;
pub mod robustness;

pub use backtest::{
    BacktestError, BacktestResult, BacktestSummary, BenchmarkPoint, CashLedgerEntry, DrawdownPoint,
    EquityPoint, FeeEntry, FillRecord, MonthlyReturn, OrderRecord, OrderSide, PositionSnapshot,
    PublicationGate,
};

/// Severity of a structured warning attached to a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    Warning,
    Critical,
}

/// A structured, typed warning carried alongside a successful result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Stable machine-readable code (e.g. `lookback_short`).
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Severity of the warning.
    pub severity: WarningSeverity,
    /// Optional structured payload (omitted when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Warning {
    /// Constructs a warning with the given code, message, and severity.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        severity: WarningSeverity,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            severity,
            details: None,
        }
    }

    /// An informational warning.
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message, WarningSeverity::Info)
    }

    /// A warning-level warning.
    pub fn warn(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message, WarningSeverity::Warning)
    }

    /// A critical warning.
    pub fn critical(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message, WarningSeverity::Critical)
    }

    /// Attaches a structured payload to the warning.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// The API error envelope (design §12.1): `code`, `message`,
/// `correlation_id`, `details`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// Stable machine-readable error code (same as `DomainError::code` for
    /// domain violations).
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Correlation id propagated through the request/log chain.
    pub correlation_id: CorrelationId,
    /// Optional structured details (omitted when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorEnvelope {
    /// Constructs an envelope from a raw code and message.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            correlation_id,
            details: None,
        }
    }

    /// Maps a typed [`DomainError`] into the envelope (stable code + message).
    pub fn from_domain(error: &DomainError, correlation_id: CorrelationId) -> Self {
        Self {
            code: error.code().to_owned(),
            message: error.to_string(),
            correlation_id,
            details: None,
        }
    }

    /// Attaches a structured payload to the envelope.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// A typed result envelope: either `data` (optionally with `warnings`) or an
/// `error` — never both, never neither in well-formed output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// The payload on success (omitted on error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Non-fatal warnings attached to the result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
    /// The error on failure (omitted on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorEnvelope>,
}

impl<T> Envelope<T> {
    /// A successful envelope carrying `data` and no warnings.
    pub fn ok(data: T) -> Self {
        Self {
            data: Some(data),
            warnings: Vec::new(),
            error: None,
        }
    }

    /// A successful envelope carrying `data` plus warnings.
    pub fn ok_with_warnings(data: T, warnings: Vec<Warning>) -> Self {
        Self {
            data: Some(data),
            warnings,
            error: None,
        }
    }

    /// A failed envelope carrying an [`ErrorEnvelope`].
    pub fn err(error: ErrorEnvelope) -> Self {
        Self {
            data: None,
            warnings: Vec::new(),
            error: Some(error),
        }
    }

    /// Whether the envelope represents a successful result.
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    /// Consumes the envelope, returning `data` or the [`ErrorEnvelope`].
    pub fn into_result(self) -> Result<T, ErrorEnvelope> {
        if let Some(error) = self.error {
            Err(error)
        } else {
            self.data.ok_or_else(|| ErrorEnvelope {
                code: "empty_envelope".to_owned(),
                message: "envelope carries neither data nor error".to_owned(),
                correlation_id: CorrelationId::generate(),
                details: None,
            })
        }
    }
}
