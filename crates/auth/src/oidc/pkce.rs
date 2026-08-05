//! PKCE (RFC 7636) primitives: S256 verifier/challenge generation.
//!
//! The verifier is 64 random alphanumeric characters (inside the 43..=128
//! unreserved-alphabet range) and the challenge is the Base64url-URL-unsafe
//! SHA-256 digest, exactly as Auth0's token endpoint expects.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

pub fn s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn verifier_is_valid(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

pub fn generate() -> PkcePair {
    let verifier = Alphanumeric.sample_string(&mut rand::rng(), 64);
    PkcePair {
        challenge: s256_challenge(&verifier),
        verifier,
    }
}

/// 32 random bytes, hex-encoded: state / nonce / opaque values.
pub fn random_hex() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_vector() {
        assert_eq!(
            s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifier_is_spec_valid() {
        for _ in 0..8 {
            let pair = generate();
            assert!(verifier_is_valid(&pair.verifier));
            assert_eq!(pair.challenge, s256_challenge(&pair.verifier));
        }
    }

    #[test]
    fn verifier_validation_edges() {
        assert!(!verifier_is_valid("short"));
        assert!(!verifier_is_valid(&"x".repeat(200)));
        assert!(!verifier_is_valid(&format!("{}!", "x".repeat(64))));
    }
}
