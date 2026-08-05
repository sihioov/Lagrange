//! Minimal RS256 JWT parsing/signature verification over ring + base64url.
//!
//! Only the well-specified subset needed to validate Auth0 ID tokens: compact
//! three-segment form, `RS256` alg, kid-based JWKS key selection, and PKCS#1
//! v1.5 SHA-256 verification. No JWT framework dependency (documented choice).

use super::{OidcError, jwks::Jwk};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};

pub const SUPPORTED_ALG: &str = "RS256";

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct JwtHeader {
    pub alg: String,
    #[serde(default)]
    pub kid: Option<String>,
}

/// Splits `h.p.s` and returns the decoded signature bytes.
pub fn split(raw: &str) -> Result<(JwtHeader, String, Vec<u8>), OidcError> {
    let mut parts = raw.split('.');
    let (Some(header_b64), Some(payload_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(OidcError::InvalidJwt(
            "expected 3 dot-separated segments".into(),
        ));
    };
    let header_json = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| OidcError::Base64Decode)?;
    let header: JwtHeader = serde_json::from_slice(&header_json)
        .map_err(|e| OidcError::InvalidJwt(format!("bad header: {e}")))?;
    let signature = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| OidcError::Base64Decode)?;
    Ok((header, payload_b64.to_string(), signature))
}

/// Verifies the RS256 signature of `raw` against the JWK's n/e.
pub fn verify_rs256(raw: &str, jwk: &Jwk) -> Result<(), OidcError> {
    let (n, e) = jwk.rsa_n_e()?;
    let mut parts = raw.split('.');
    let signing_input = match (parts.next(), parts.next()) {
        (Some(h), Some(p)) => format!("{h}.{p}"),
        _ => {
            return Err(OidcError::InvalidJwt(
                "expected 3 dot-separated segments".into(),
            ));
        }
    };
    let (_, _, signature) = split(raw)?;
    let components = RsaPublicKeyComponents { n: &n, e: &e };
    components
        .verify(
            &RSA_PKCS1_2048_8192_SHA256,
            signing_input.as_bytes(),
            &signature,
        )
        .map_err(|_| OidcError::SignatureInvalid)
}

/// Decodes the payload segment (already base64url) into typed claims.
pub fn decode_payload(payload_b64: &str) -> Result<serde_json::Value, OidcError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| OidcError::Base64Decode)?;
    serde_json::from_slice(&bytes).map_err(|e| OidcError::InvalidJwt(format!("bad payload: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_rejects_malformed() {
        for bad in ["", "a.b", "a.b.c.d", "!!!.!!!.!!!"] {
            assert!(split(bad).is_err(), "{bad:?} must error");
        }
    }
}
