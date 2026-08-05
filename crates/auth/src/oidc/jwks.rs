//! JWKS (RFC 7517) types: parsing, key selection, RSA n/e extraction.

use super::OidcError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Jwk {
    pub kty: String,
    pub kid: Option<String>,
    #[serde(rename = "use", default)]
    pub use_: Option<String>,
    pub alg: Option<String>,
    pub n: Option<String>,
    pub e: Option<String>,
}

impl Jwks {
    pub fn parse(raw: &str) -> Result<Self, OidcError> {
        serde_json::from_str(raw).map_err(|e| OidcError::InvalidJwks(e.to_string()))
    }

    pub fn key_for_kid(&self, kid: &str) -> Option<&Jwk> {
        self.keys.iter().find(|k| k.kid.as_deref() == Some(kid))
    }

    /// The only usable key when the JWKS carries a single entry without a kid.
    pub fn sole_key(&self) -> Option<&Jwk> {
        (self.keys.len() == 1 && self.keys[0].kid.is_none()).then(|| &self.keys[0])
    }
}

impl Jwk {
    /// The signature-use key must be RSA; n/e are required, big-endian bytes.
    pub fn rsa_n_e(&self) -> Result<(Vec<u8>, Vec<u8>), OidcError> {
        if self.kty != "RSA" {
            return Err(OidcError::UnsupportedKeyType(self.kty.clone()));
        }
        if let Some(use_) = &self.use_
            && use_ != "sig"
        {
            return Err(OidcError::KeyNotForSigning(use_.clone()));
        }
        let n = self
            .n
            .as_deref()
            .ok_or_else(|| OidcError::MissingKeyMaterial("n".to_string()))?;
        let e = self
            .e
            .as_deref()
            .ok_or_else(|| OidcError::MissingKeyMaterial("e".to_string()))?;
        let n = URL_SAFE_NO_PAD
            .decode(n)
            .map_err(|_| OidcError::Base64Decode)?;
        let e = URL_SAFE_NO_PAD
            .decode(e)
            .map_err(|_| OidcError::Base64Decode)?;
        Ok((n, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auth0_style_jwks() {
        let raw =
            r#"{"keys":[{"kty":"RSA","use":"sig","kid":"abc","alg":"RS256","n":"x","e":"AQAB"}]}"#;
        let jwks = Jwks::parse(raw).expect("parses");
        assert_eq!(
            jwks.key_for_kid("abc").unwrap().alg.as_deref(),
            Some("RS256")
        );
        assert_eq!(jwks.key_for_kid("other"), None);
    }

    #[test]
    fn rsa_material_extracted() {
        let raw = r#"{"keys":[{"kty":"RSA","use":"sig","kid":"abc","alg":"RS256","n":"AQAB","e":"AQAB"}]}"#;
        let jwks = Jwks::parse(raw).unwrap();
        let (n, e) = jwks.key_for_kid("abc").unwrap().rsa_n_e().expect("n/e");
        assert_eq!(n, vec![1, 0, 1]);
        assert_eq!(e, vec![1, 0, 1]);
    }

    #[test]
    fn non_rsa_or_encryption_key_is_rejected() {
        let ec = r#"{"keys":[{"kty":"EC","kid":"abc","crv":"P-256","x":"x","y":"y"}]}"#;
        let jwks = Jwks::parse(ec).unwrap();
        assert!(matches!(
            jwks.key_for_kid("abc").unwrap().rsa_n_e(),
            Err(OidcError::UnsupportedKeyType(_))
        ));
        let enc = r#"{"keys":[{"kty":"RSA","use":"enc","kid":"abc","n":"x","e":"AQAB"}]}"#;
        let jwks = Jwks::parse(enc).unwrap();
        assert!(matches!(
            jwks.key_for_kid("abc").unwrap().rsa_n_e(),
            Err(OidcError::KeyNotForSigning(_))
        ));
    }

    #[test]
    fn malformed_jwks_is_typed_error() {
        assert!(Jwks::parse("not json").is_err());
        assert!(
            Jwks::parse(r#"{"keys":[]}"#)
                .unwrap()
                .key_for_kid("x")
                .is_none()
        );
    }
}
