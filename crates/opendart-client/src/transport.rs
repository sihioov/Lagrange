//! The HTTP boundary. [`LiveTransport`] is the only implementation that
//! touches the network; `client`'s tests substitute [`Transport`]
//! implementations that never do, so the retry/timeout/status logic there
//! is verifiable offline.
//!
//! This module is also the leak choke point at the transport level: a
//! `reqwest::Error`'s `Display` renders the full outbound URL, including
//! the `crtfc_key` query parameter, so no `reqwest::Error` value -- and
//! nothing built by formatting one -- is allowed to leave [`Transport::send`].
//! Every item here is crate-private: `Transport`, `HttpRequest`, and
//! `LiveTransport` never appear in this crate's public interface (see
//! `client::OpenDartClient`, which is a concrete, non-generic struct for
//! exactly this reason).

use std::time::Duration;

use crate::error::OpenDartTransportError;

/// A fully-formed outbound request: an allowlisted path plus query
/// (including the credential, appended by `client` before this is built).
pub(crate) struct HttpRequest {
    pub path: &'static str,
    pub query: Vec<(String, String)>,
}

/// A raw response: status and body bytes, uninterpreted. `client` decides
/// what a status or a body fragment means; this module never parses either.
pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// How a request failed, independent of the HTTP library. Exists so retry
/// classification (`status::is_retryable_failure`) is unit-testable without
/// a network, and so the `reqwest::Error` that produced it never has to
/// leave this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    NeverSent,
    TimedOut,
    Indeterminate,
    UnreadableBody,
}

/// Pure, testable failure -> typed-error mapping. Never touches a
/// `reqwest::Error`, only the coarse [`Failure`] classification.
pub(crate) fn classify(failure: Failure) -> OpenDartTransportError {
    match failure {
        Failure::NeverSent => OpenDartTransportError::NeverSent,
        Failure::TimedOut => OpenDartTransportError::TimedOut,
        Failure::Indeterminate => OpenDartTransportError::Indeterminate,
        Failure::UnreadableBody => OpenDartTransportError::UnreadableBody,
    }
}

/// Reduces the two safe `reqwest::Error` classification signals to one coarse
/// request failure. A connect signal takes precedence because it proves the
/// request never left the process, even if it was caused by a connect timeout.
fn classify_send_error(is_connect: bool, is_timeout: bool) -> Failure {
    if is_connect {
        Failure::NeverSent
    } else if is_timeout {
        Failure::TimedOut
    } else {
        Failure::Indeterminate
    }
}

/// Sends an already-built request and returns a raw response or a coarse
/// failure. Implementations must never let a `reqwest::Error` (or its
/// formatted text) escape.
pub(crate) trait Transport: Send + Sync {
    fn send(
        &self,
        request: &HttpRequest,
    ) -> impl std::future::Future<Output = Result<HttpResponse, Failure>> + Send;
}

/// The only OpenDART host this client speaks to.
pub(crate) const OPENDART_BASE_URL: &str = "https://opendart.fss.or.kr";

/// The real network transport. Built with an explicit connect timeout, an
/// explicit read timeout, and redirects disabled -- a 3xx response would
/// otherwise resend the keyed URL to a host this crate did not choose.
pub(crate) struct LiveTransport {
    client: reqwest::Client,
    base_url: String,
}

impl LiveTransport {
    pub fn new(
        base_url: impl Into<String>,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, OpenDartTransportError> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(read_timeout)
            // Redirects are never followed: a 3xx here would resend
            // `crtfc_key` to a URL this crate did not choose. `reqwest`
            // still returns the 3xx response itself (not an error) with
            // this policy, so `client::ClientCore::get` classifies it via
            // `status::classify_status` like any other status.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            // Deliberately discard `e`: its `Display` can render proxy/TLS
            // config details we do not want to promise never include a
            // credential-adjacent string. The coarse variant is enough to
            // act on (fail startup); it is never useful to retry.
            .map_err(|_e| OpenDartTransportError::ClientBuildFailed)?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }
}

impl Transport for LiveTransport {
    async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, Failure> {
        let url = format!("{}{}", self.base_url, request.path);
        let outbound = self.client.get(&url).query(&request.query);
        let response = match outbound.send().await {
            Ok(r) => r,
            Err(e) => {
                // `is_connect()` is the only reqwest signal that proves the
                // request never left the process. Everything else about
                // `e` -- most importantly its own `Display`, which would
                // render the full keyed URL -- is discarded right here; it
                // is never captured into a `String` or logged.
                return Err(classify_send_error(e.is_connect(), e.is_timeout()));
            }
        };
        let status = response.status().as_u16();
        let body = match response.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_e) => return Err(Failure::UnreadableBody),
        };
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_each_failure_to_its_own_terminal_variant() {
        assert_eq!(
            classify(Failure::NeverSent),
            OpenDartTransportError::NeverSent
        );
        assert_eq!(
            classify(Failure::TimedOut),
            OpenDartTransportError::TimedOut
        );
        assert_eq!(
            classify(Failure::Indeterminate),
            OpenDartTransportError::Indeterminate
        );
        assert_eq!(
            classify(Failure::UnreadableBody),
            OpenDartTransportError::UnreadableBody
        );
    }

    #[test]
    fn send_error_signals_preserve_only_proven_never_sent_failures() {
        assert_eq!(classify_send_error(true, true), Failure::NeverSent);
        assert_eq!(classify_send_error(true, false), Failure::NeverSent);
        assert_eq!(classify_send_error(false, true), Failure::TimedOut);
        assert_eq!(classify_send_error(false, false), Failure::Indeterminate);
    }

    #[test]
    fn live_transport_builds_with_explicit_timeouts_and_no_redirects() {
        // Offline-reachable assertion for the redirect policy: construction
        // succeeds with a deny-redirect client. The behavioural half of the
        // guarantee (a 3xx is treated as terminal, never followed) is
        // exercised in `status::tests::redirect_is_terminal` and in
        // `client`'s fake-transport tests, since asserting the live
        // `reqwest::Client`'s policy field directly is not exposed by the
        // library and would require a network call to observe.
        let transport = LiveTransport::new(
            OPENDART_BASE_URL,
            Duration::from_millis(500),
            Duration::from_secs(5),
        );
        assert!(transport.is_ok());
    }
}
