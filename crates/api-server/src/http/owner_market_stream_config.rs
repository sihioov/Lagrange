//! Immutable, provider-free configuration for the schema-2 read surface.

use std::sync::Arc;

use axum::http::{HeaderMap, Uri, header};
use collectors::intraday_quotes::IntradaySessionWindowContract;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MarketStreamReadConfig {
    #[default]
    Disabled,
    OwnerOnly(MarketStreamReadPins),
}

/// These are non-secret identities. The API has no provider credential or
/// writable runtime-domain path and cannot construct a broker transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketStreamReadPins {
    origin: String,
    authority: String,
    pub(crate) credential_slot_id: Uuid,
    pub(crate) grant_id: Uuid,
    pub(crate) contract_sha256: String,
    pub(crate) window: Option<Arc<IntradaySessionWindowContract>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidStreamConfiguration;

impl MarketStreamReadPins {
    pub fn new(
        origin: String,
        credential_slot_id: Uuid,
        grant_id: Uuid,
        contract_sha256: String,
        window: Option<Arc<IntradaySessionWindowContract>>,
    ) -> Result<Self, InvalidStreamConfiguration> {
        let uri: Uri = origin.parse().map_err(|_| InvalidStreamConfiguration)?;
        let authority = uri.authority().ok_or(InvalidStreamConfiguration)?.as_str();
        if origin.len() > 256
            || uri.scheme_str() != Some("https")
            || uri.host().is_none_or(str::is_empty)
            || authority.contains('@')
            || authority != authority.to_ascii_lowercase()
            || origin != format!("https://{authority}")
            || credential_slot_id.is_nil()
            || grant_id.is_nil()
            || contract_sha256.len() != 64
            || !contract_sha256
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(InvalidStreamConfiguration);
        }
        Ok(Self {
            authority: authority.to_owned(),
            origin,
            credential_slot_id,
            grant_id,
            contract_sha256,
            window,
        })
    }

    /// Host is matched to a deployment pin, never reflected into an allowlist.
    /// Forwarded headers are not accepted as an origin authority.
    pub(crate) fn allows_request(&self, headers: &HeaderMap, uri: &Uri) -> bool {
        let one = |name| {
            let values = headers.get_all(name);
            let mut iter = values.iter();
            let first = iter.next();
            if iter.next().is_some() {
                return Err(());
            }
            first.map(|v| v.to_str().map_err(|_| ())).transpose()
        };
        let Ok(origin) = one(header::ORIGIN) else {
            return false;
        };
        if origin.is_some_and(|v| v != self.origin) {
            return false;
        }
        let Ok(site) = one(axum::http::HeaderName::from_static("sec-fetch-site")) else {
            return false;
        };
        if site.is_some_and(|v| v != "same-origin" && v != "none") {
            return false;
        }
        let Ok(host) = one(header::HOST) else {
            return false;
        };
        if uri
            .authority()
            .is_some_and(|value| value.as_str() != self.authority)
        {
            return false;
        }
        host.or_else(|| uri.authority().map(|v| v.as_str())) == Some(self.authority.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn pins() -> MarketStreamReadPins {
        MarketStreamReadPins::new(
            "https://quotes.example".into(),
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "a".repeat(64),
            None,
        )
        .unwrap()
    }

    #[test]
    fn only_an_exact_https_origin_and_nonsecret_canonical_pins_are_accepted() {
        for origin in [
            "http://quotes.example",
            "https://quotes.example/",
            "https://quotes.example?q=x",
            "https://x@quotes.example",
            "https://QUOTES.example",
            "null",
        ] {
            assert!(
                MarketStreamReadPins::new(
                    origin.into(),
                    Uuid::from_u128(1),
                    Uuid::from_u128(2),
                    "a".repeat(64),
                    None
                )
                .is_err()
            );
        }
        assert!(
            MarketStreamReadPins::new(
                "https://quotes.example".into(),
                Uuid::nil(),
                Uuid::from_u128(2),
                "a".repeat(64),
                None
            )
            .is_err()
        );
        assert!(
            MarketStreamReadPins::new(
                "https://quotes.example".into(),
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                "A".repeat(64),
                None
            )
            .is_err()
        );
        assert!(pins().window.is_none());
    }

    #[test]
    fn origin_metadata_and_host_must_all_agree_without_forwarded_fallback() {
        let pins = pins();
        let uri: Uri = "/api/v1/market-stream".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("quotes.example"));
        assert!(pins.allows_request(&headers, &uri));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://quotes.example"),
        );
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(pins.allows_request(&headers, &uri));
        for site in ["cross-site", "same-site", "invalid"] {
            headers.insert("sec-fetch-site", HeaderValue::from_str(site).unwrap());
            assert!(!pins.allows_request(&headers, &uri));
        }
        headers.remove("sec-fetch-site");
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("https://quotes.example"),
        );
        assert!(!pins.allows_request(&headers, &uri));
        headers.remove(header::ORIGIN);
        headers.insert(header::HOST, HeaderValue::from_static("attacker.example"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("quotes.example"),
        );
        assert!(!pins.allows_request(&headers, &uri));
    }
}
