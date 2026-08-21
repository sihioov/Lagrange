use crate::credential::CredentialError;

/// Coarse, safe transport failures. No URL, query, response body, or
/// provider prose crosses this boundary because the keyed request URL must not
/// appear in diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DataGoTransportError {
    #[error("data.go client construction failed")]
    ClientBuildFailed,
    #[error("data.go credential configuration failed: {0}")]
    Credential(CredentialError),
    #[error("data.go request was not sent")]
    NeverSent,
    #[error("data.go request timed out")]
    TimedOut,
    #[error("data.go request outcome was indeterminate")]
    Indeterminate,
    #[error("data.go response body could not be read")]
    UnreadableBody,
    #[error("data.go response body exceeded the permitted bound")]
    ResponseTooLarge,
    #[error("data.go endpoint redirected with HTTP status {status}")]
    Redirected { status: u16 },
    #[error("data.go endpoint returned HTTP status {status}")]
    UnexpectedStatus { status: u16 },
    #[error("data.go query configuration is invalid")]
    InvalidQuery,
    #[error("data.go query date is not a valid YYYYMMDD calendar date")]
    InvalidDate,
    #[error("data.go query ISIN is outside the approved fixed ETF11 universe")]
    UnapprovedIsin,
}
