//! CSRF synchronizer tokens (per-session, hashed at rest).
//!
//! Each session is minted with a random token; the SHA-256 hash is stored on
//! the session record (the same hashing discipline as the cookie value), the
//! plaintext reaches the browser exactly once over TLS (login response header
//! or the session-authenticated `GET /auth/csrf` endpoint). Mutations require
//! the header/field to echo the token; verification is constant-time.

use subtle::ConstantTimeEq;

use crate::oidc::pkce::random_hex;
use crate::sessions::cookie;

pub fn generate_token() -> String {
    random_hex()
}

pub fn hash_token(token: &str) -> String {
    cookie::hash(token)
}

/// Constant-time check that `presented` hashes to the stored value.
pub fn verify(stored_hash: &str, presented: &str) -> bool {
    let presented_hash = cookie::hash(presented);
    let stored = stored_hash.as_bytes();
    let presented = presented_hash.as_bytes();
    stored.len() == presented.len() && bool::from(stored.ct_eq(presented))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_token_verifies_and_wrong_is_denied() {
        let token = generate_token();
        let hash = hash_token(&token);
        assert!(verify(&hash, &token));
        assert!(!verify(&hash, "wrong-token"));
        assert!(!verify(&hash, ""));
        assert!(!verify("", &token));
    }

    #[test]
    fn tokens_are_unique_and_entropic() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64, "32 bytes hex");
    }
}
