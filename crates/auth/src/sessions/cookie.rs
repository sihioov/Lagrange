//! Opaque first-party session cookie: `__Host-lagrange_session`.
//!
//! Attributes follow NFR-SEC-004 and the `__Host-` prefix rules: `Secure`,
//! `HttpOnly`, `SameSite=Lax`, `Path=/`, and NO `Domain` attribute (host-only),
//! so the cookie cannot be set for a parent domain or read by script.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};

pub const NAME: &str = "__Host-lagrange_session";
/// Short session: re-login rather than browser refresh tokens.
pub const TTL_SECS: i64 = 1800;

/// 32 random bytes, base64url (43 chars) - the opaque bearer of the session.
pub fn generate_value() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256 hex of the opaque value; the ONLY form stored in the session store.
pub fn hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)
}

pub fn expires_rfc1123(unix_secs: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(unix_secs, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

pub fn set_cookie(value: &str, expires_at_secs: i64) -> String {
    let max_age = expires_at_secs
        .checked_sub(chrono::Utc::now().timestamp())
        .unwrap_or(0);
    format!(
        "{NAME}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}; Expires={}",
        expires_rfc1123(expires_at_secs)
    )
}

pub fn clear_cookie() -> String {
    format!(
        "{NAME}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
    )
}

/// Extracts the value of `name` from a Cookie request header (first match).
pub fn parse(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let part = part.trim();
        let (k, v) = part.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_is_opaque_entropic_and_unique() {
        let a = generate_value();
        let b = generate_value();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
    }

    #[test]
    fn hash_is_hex_and_deterministic() {
        assert_eq!(hash("value"), hash("value"));
        assert_eq!(hash("value").len(), 64);
        assert_ne!(hash("value"), hash("other"));
    }

    #[test]
    fn set_cookie_has_host_prefix_attributes_and_no_domain() {
        let header = set_cookie("abc", 1_900_000_000);
        assert!(header.starts_with(&format!("{NAME}=abc;")));
        for attr in [
            "Path=/",
            "Secure",
            "HttpOnly",
            "SameSite=Lax",
            "Max-Age=",
            "Expires=",
        ] {
            assert!(header.contains(attr), "missing {attr} in {header}");
        }
        assert!(
            !header.contains("Domain="),
            "host-only cookie must not set Domain"
        );
    }

    #[test]
    fn clear_cookie_zeroes_max_age() {
        let header = clear_cookie();
        assert!(header.contains("Max-Age=0"));
        assert!(header.contains("Expires=Thu, 01 Jan 1970"));
    }

    #[test]
    fn parse_finds_named_cookie_only() {
        let h = "other=1; __Host-lagrange_session=opaque-xyz; third=3";
        assert_eq!(parse(h, NAME).as_deref(), Some("opaque-xyz"));
        assert_eq!(parse(h, "nope"), None);
    }

    #[test]
    fn rfc1123_format() {
        assert_eq!(expires_rfc1123(0), "Thu, 01 Jan 1970 00:00:00 GMT");
    }
}
