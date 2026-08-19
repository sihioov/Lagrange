//! Pure classification of transport outcomes into retry/terminal
//! dispositions. Everything here is a plain function over already-coarse
//! types (`StatusClass`, `Failure`) -- no I/O, no `reqwest` types -- so it
//! is fully unit-testable without a network.
//!
//! OpenDART's own `020`/`021` application statuses are not classified here:
//! `client::detect_terminal_application_status` maps them straight to a
//! terminal `OpenDartTransportError` and the retry loop never calls back
//! into this module for them, so there is no disposition function to test
//! in isolation -- their terminality is exercised end-to-end instead, in
//! `client::tests::application_status_020_is_terminal_with_no_retry`.

use crate::transport::Failure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    Success,
    Redirect,
    ClientError,
    ServerError,
}

/// Buckets a raw status code. `2xx` -> success, `3xx` -> redirect (always
/// terminal here -- redirects are never followed), `5xx` -> the only
/// retryable status bucket, everything else -> client error (terminal).
pub fn classify_status(status: u16) -> StatusClass {
    match status {
        200..=299 => StatusClass::Success,
        300..=399 => StatusClass::Redirect,
        500..=599 => StatusClass::ServerError,
        _ => StatusClass::ClientError,
    }
}

/// Only a `5xx` status bucket is retryable. `3xx` and `4xx` are always
/// terminal.
pub fn is_retryable_status(class: StatusClass) -> bool {
    matches!(class, StatusClass::ServerError)
}

/// Only a connect failure that proves the request never reached the server
/// is retryable. A timeout may have already reached the server; an
/// unreadable body definitely did -- retrying either risks a duplicate
/// side effect on OpenDART's end, so both are terminal.
pub fn is_retryable_failure(failure: Failure) -> bool {
    matches!(failure, Failure::NeverSent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_is_not_retryable() {
        assert_eq!(classify_status(200), StatusClass::Success);
        assert_eq!(classify_status(201), StatusClass::Success);
        assert!(!is_retryable_status(classify_status(200)));
    }

    #[test]
    fn redirect_is_terminal() {
        assert_eq!(classify_status(301), StatusClass::Redirect);
        assert_eq!(classify_status(308), StatusClass::Redirect);
        assert!(!is_retryable_status(StatusClass::Redirect));
    }

    #[test]
    fn client_error_is_terminal() {
        assert_eq!(classify_status(404), StatusClass::ClientError);
        assert_eq!(classify_status(401), StatusClass::ClientError);
        assert!(!is_retryable_status(StatusClass::ClientError));
    }

    #[test]
    fn server_error_is_retryable() {
        assert_eq!(classify_status(500), StatusClass::ServerError);
        assert_eq!(classify_status(503), StatusClass::ServerError);
        assert!(is_retryable_status(StatusClass::ServerError));
    }

    #[test]
    fn never_sent_is_retryable_but_other_failures_are_not() {
        assert!(is_retryable_failure(Failure::NeverSent));
        assert!(!is_retryable_failure(Failure::TimedOut));
        assert!(!is_retryable_failure(Failure::UnreadableBody));
    }
}
