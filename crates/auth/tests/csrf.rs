//! Todo 22 RED suite: CSRF synchronizer tokens - missing/wrong -> denial.

use auth::csrf::{generate_token, hash_token, verify};

#[test]
fn correct_token_verifies() {
    let token = generate_token();
    assert!(verify(&hash_token(&token), &token));
}

#[test]
fn missing_or_wrong_token_is_denied() {
    let token = generate_token();
    assert!(!verify(&hash_token(&token), ""));
    assert!(!verify(&hash_token(&token), "wrong-token"));
    assert!(!verify(&hash_token(&token), &generate_token()));
    assert!(!verify("", &token));
}

#[test]
fn tokens_are_unique() {
    let a = generate_token();
    let b = generate_token();
    assert_ne!(a, b);
    assert_ne!(hash_token(&a), hash_token(&b));
}

#[test]
fn token_is_not_the_session_cookie() {
    // Synchronizer tokens are 64-hex (32 random bytes) and independent of the
    // opaque cookie value; they must never be derived from it.
    let cookie_value = "opaque-cookie-value";
    let token = generate_token();
    assert_ne!(token, cookie_value);
    assert_ne!(token, hash_token(cookie_value));
}
