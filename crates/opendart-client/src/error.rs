//! The transport-level error type this crate returns to callers.
//!
//! Every variant is deliberately coarse: a class of failure plus, at most,
//! a numeric status code. This is the choke point for the crate's central
//! leak rule -- `reqwest::Error`'s `Display` renders the full outbound URL
//! (`crtfc_key` included), so no `reqwest::Error`, and nothing that wraps or
//! formats one, is allowed to reach a variant here. None of these variants
//! has a `String`/bytes field at all, so there is structurally nowhere for a
//! key, a URL, or a response body to be attached even by mistake.
//!
//! See the `no_variant_leaks_sensitive_data` test at the bottom of this file
//! for the exhaustive check.

use crate::credential::CredentialError;

/// The two OpenDART application-level statuses that must never be retried.
/// The documented daily quota that triggers `020` is unconfirmed upstream,
/// so this client must never probe or back off into it -- it fails closed
/// on the first occurrence instead of guessing at a retry schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationStatus {
    /// OpenDART status "020": request-limit exceeded.
    RequestLimitExceeded,
    /// OpenDART status "021": company-count exceeded.
    CompanyCountExceeded,
}

/// Everything that can go wrong making an OpenDART request, reduced to the
/// coarse classes safe to display: no URL, no query string, no response
/// body, no `reqwest` error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OpenDartTransportError {
    /// The requested path is not on the fixed allowlist. Rejected before
    /// any request is constructed.
    #[error("requested path is not on the allowed OpenDART read surface")]
    PathNotAllowed,

    /// Loading `crtfc_key` from disk failed; see `CredentialError` for why.
    #[error("credential unavailable: {0}")]
    Credential(#[from] CredentialError),

    /// The network client could not be constructed (e.g. bad timeout/TLS
    /// config). Carries no inner error text.
    #[error("network client could not be constructed")]
    ClientBuildFailed,

    /// The request never left the process (DNS/connect failure). The only
    /// failure class this client retries.
    #[error("request could not be sent")]
    NeverSent,

    /// The request was sent but no response arrived before the configured
    /// timeout. Not retried: the request may have already reached the
    /// server.
    #[error("no response arrived before the configured timeout elapsed")]
    TimedOut,

    /// A request failed without a response, but the HTTP client could not
    /// establish that it was never sent or that it timed out. Not retried.
    #[error("request failed without a response")]
    Indeterminate,

    /// A response arrived but its body could not be read to completion.
    /// Not retried: the request reached the server.
    #[error("response body could not be read to completion")]
    UnreadableBody,

    /// OpenDART returned a 3xx. Redirects are never followed (resending a
    /// keyed URL to another host is a leak), so this is always terminal.
    #[error("server responded with a redirect (status {status}); redirects are not followed")]
    Redirected { status: u16 },

    /// A non-2xx, non-3xx status. Carries only the numeric code. 5xx is
    /// retried up to the bound; everything else is terminal.
    #[error("server responded with status {status}")]
    UnexpectedStatus { status: u16 },

    /// OpenDART's own application-level status was `020` or `021`. Always
    /// terminal -- see [`ApplicationStatus`].
    #[error("OpenDART application status is terminal and will not be retried: {0:?}")]
    ApplicationStatus(ApplicationStatus),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Substrings that must never appear in any rendering of this crate's
    /// error type: a sentinel standing in for a real key value, the query
    /// parameter name, anything resembling "http" (which would suggest a
    /// URL or a raw `reqwest`/scheme fragment leaked through), and the
    /// OpenDART host itself. Checked case-insensitively so a variant naming
    /// mistake (e.g. `HttpStatus`) can't slip past a case-sensitive check.
    const FORBIDDEN: [&str; 4] = [
        "sk-lagrange-forbidden-sentinel",
        "crtfc_key",
        "http",
        "opendart.fss.or.kr",
    ];

    fn assert_no_leak(err: &OpenDartTransportError) {
        let display = format!("{err}").to_lowercase();
        let debug = format!("{err:?}").to_lowercase();
        for needle in FORBIDDEN {
            assert!(
                !display.contains(needle),
                "Display leaked {needle:?}: {display}"
            );
            assert!(!debug.contains(needle), "Debug leaked {needle:?}: {debug}");
        }
    }

    #[test]
    fn no_variant_leaks_sensitive_data() {
        let samples = [
            OpenDartTransportError::PathNotAllowed,
            OpenDartTransportError::Credential(CredentialError::EnvVarMissing),
            OpenDartTransportError::Credential(CredentialError::FileNotFound),
            OpenDartTransportError::Credential(CredentialError::FileUnreadable),
            OpenDartTransportError::Credential(CredentialError::FileEmpty),
            OpenDartTransportError::ClientBuildFailed,
            OpenDartTransportError::NeverSent,
            OpenDartTransportError::TimedOut,
            OpenDartTransportError::Indeterminate,
            OpenDartTransportError::UnreadableBody,
            OpenDartTransportError::Redirected { status: 301 },
            OpenDartTransportError::Redirected { status: 308 },
            OpenDartTransportError::UnexpectedStatus { status: 404 },
            OpenDartTransportError::UnexpectedStatus { status: 500 },
            OpenDartTransportError::ApplicationStatus(ApplicationStatus::RequestLimitExceeded),
            OpenDartTransportError::ApplicationStatus(ApplicationStatus::CompanyCountExceeded),
        ];

        // Compile-time exhaustiveness guarantee: this match has no wildcard
        // arm over either enum, so adding a variant to `OpenDartTransportError`
        // or to `CredentialError` without updating `samples` above fails to
        // build, rather than silently under-testing.
        fn assert_exhaustive(e: &OpenDartTransportError) {
            match e {
                OpenDartTransportError::PathNotAllowed => {}
                OpenDartTransportError::Credential(c) => match c {
                    CredentialError::EnvVarMissing
                    | CredentialError::FileNotFound
                    | CredentialError::FileUnreadable
                    | CredentialError::FileEmpty => {}
                },
                OpenDartTransportError::ClientBuildFailed => {}
                OpenDartTransportError::NeverSent => {}
                OpenDartTransportError::TimedOut => {}
                OpenDartTransportError::Indeterminate => {}
                OpenDartTransportError::UnreadableBody => {}
                OpenDartTransportError::Redirected { .. } => {}
                OpenDartTransportError::UnexpectedStatus { .. } => {}
                OpenDartTransportError::ApplicationStatus(a) => match a {
                    ApplicationStatus::RequestLimitExceeded
                    | ApplicationStatus::CompanyCountExceeded => {}
                },
            }
        }

        for sample in &samples {
            assert_exhaustive(sample);
            assert_no_leak(sample);
        }
    }
}
