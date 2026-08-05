//! Auth0 simulator: a fake OIDC provider proving the full login contract
//! without a real tenant (BLOCKED_EXTERNAL: no Auth0 tenant/credentials exist
//! on this host). The simulator speaks the same wire contract the real tenant
//! does - PKCE S256 challenge/verifier, exact redirect URI, single-use auth
//! codes, RS256-signed ID tokens served through a JWKS endpoint - so the
//! protocol core and the Axum router are exercised end-to-end.
//!
//! Real-tenant verification is the `vendor`-tagged suite in
//! `crates/auth/tests/vendor_auth0.rs`: ignored by default, it must run before
//! any production release gate and is never silently skipped (it fails loudly
//! when credentials are absent).
//!
//! Compiled unconditionally so integration tests and downstream crates can
//! drive the contract without enabling features; it is inert test-support
//! code - no network, no credentials.

use crate::oidc::pkce::random_hex;
use crate::oidc::{
    AuthorizeRequest, OidcTransport, TokenRequest, TokenResponse, TransportError, jwks,
};
use crate::testkey::{TEST_RSA_PRIVATE_PKCS8_DER, TEST_RSA_PUBLIC_SPKI_DER};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

pub const SIM_KID: &str = "lagrange-test-kid-1";
pub const SIM_AUDIENCE: &str = "https://api.lagrange.local";
const CODE_TTL_SECS: i64 = 60;

/// Minimal DER walker: extracts the RSA modulus and exponent from a
/// SubjectPublicKeyInfo document (SEQUENCE { SEQUENCE { OID, NULL },
/// BIT STRING { SEQUENCE { INTEGER n, INTEGER e } } }). TLV items are
/// returned as `(tag, content_start, end)` with end = start + content_len.
fn spki_rsa_n_e(der: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    fn read_tlv(data: &[u8], offset: usize) -> Result<(u8, usize, usize), String> {
        let tag = *data.get(offset).ok_or("truncated tag")?;
        let len_byte = *data.get(offset + 1).ok_or("truncated length")?;
        let (len, head) = match len_byte {
            0x81 => {
                let l = *data.get(offset + 2).ok_or("truncated long length")? as usize;
                (l, 3)
            }
            0x82 => {
                let hi = *data.get(offset + 2).ok_or("truncated long length")? as usize;
                let lo = *data.get(offset + 3).ok_or("truncated long length")? as usize;
                ((hi << 8) | lo, 4)
            }
            l if l < 0x80 => (l as usize, 2),
            _ => return Err("unsupported length encoding".into()),
        };
        let start = offset + head;
        let end = start.checked_add(len).ok_or("length overflow")?;
        if end > data.len() {
            return Err("TLV runs past the buffer".into());
        }
        Ok((tag, start, end))
    }
    fn integer(data: &[u8], offset: usize) -> Result<(Vec<u8>, usize), String> {
        let (tag, start, end) = read_tlv(data, offset)?;
        if tag != 0x02 {
            return Err("expected INTEGER".into());
        }
        let mut bytes = data[start..end].to_vec();
        if bytes.first() == Some(&0) {
            bytes.remove(0);
        }
        Ok((bytes, end))
    }
    let (seq_tag, _, _) = read_tlv(der, 0)?;
    if seq_tag != 0x30 {
        return Err("expected top-level SEQUENCE".into());
    }
    let (_, alg_start, _) = read_tlv(der, 0)?;
    let (alg_tag, _, alg_end) = read_tlv(der, alg_start)?;
    if alg_tag != 0x30 {
        return Err("expected algorithm SEQUENCE".into());
    }
    let (bs_tag, bs_start, _) = read_tlv(der, alg_end)?;
    if bs_tag != 0x03 {
        return Err("expected BIT STRING".into());
    }
    let inner = bs_start + 1; // unused-bits byte
    let (seq_tag, seq_start, _) = read_tlv(der, inner)?;
    if seq_tag != 0x30 {
        return Err("expected key SEQUENCE".into());
    }
    let (n, after_n) = integer(der, seq_start)?;
    let (e, _) = integer(der, after_n)?;
    Ok((n, e))
}

struct IssuedCode {
    claims: Value,
    verifier: String,
    redirect_uri: String,
    expires_at_secs: i64,
}

pub struct Simulator {
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub kid: String,
    key: RsaKeyPair,
    n: Vec<u8>,
    e: Vec<u8>,
    codes: Mutex<HashMap<String, IssuedCode>>,
}

impl Simulator {
    pub fn new(issuer: &str, client_id: &str, redirect_uri: &str) -> Self {
        let key = RsaKeyPair::from_pkcs8(TEST_RSA_PRIVATE_PKCS8_DER).expect("test rsa key loads");
        let (n, e) = spki_rsa_n_e(TEST_RSA_PUBLIC_SPKI_DER).expect("spki n/e extraction");
        Self {
            issuer: issuer.to_string(),
            client_id: client_id.to_string(),
            redirect_uri: redirect_uri.to_string(),
            kid: SIM_KID.to_string(),
            key,
            n,
            e,
            codes: Mutex::new(HashMap::new()),
        }
    }

    pub fn jwks(&self) -> jwks::Jwks {
        let key = jwks::Jwk {
            kty: "RSA".to_string(),
            kid: Some(self.kid.clone()),
            use_: Some("sig".to_string()),
            alg: Some("RS256".to_string()),
            n: Some(URL_SAFE_NO_PAD.encode(&self.n)),
            e: Some(URL_SAFE_NO_PAD.encode(&self.e)),
        };
        jwks::Jwks { keys: vec![key] }
    }

    fn sign(&self, header: &Value, claims: &Value) -> String {
        let header_json = serde_json::to_vec(header).expect("header json");
        let claims_json = serde_json::to_vec(claims).expect("claims json");
        let header_b64 = URL_SAFE_NO_PAD.encode(&header_json);
        let claims_b64 = URL_SAFE_NO_PAD.encode(&claims_json);
        let signing_input = format!("{header_b64}.{claims_b64}");
        let mut signature = vec![0u8; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .expect("rsa sign");
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    pub fn sign_id_token(&self, claims: &Value) -> String {
        self.sign_id_token_with_kid(&self.kid, claims)
    }

    pub fn sign_id_token_with_kid(&self, kid: &str, claims: &Value) -> String {
        self.sign(
            &serde_json::json!({"alg": "RS256", "typ": "JWT", "kid": kid}),
            claims,
        )
    }

    pub fn sign_raw(&self, header: Value, claims: Value) -> String {
        self.sign(&header, &claims)
    }

    /// "Logs the user in" at the provider: mints a single-use auth code bound
    /// to the verifier and redirect URI, storing the claims to sign later.
    pub fn issue_code(&self, claims: Value, verifier: &str) -> String {
        let code = format!("sim-code-{}", random_hex());
        self.codes.lock().unwrap().insert(
            code.clone(),
            IssuedCode {
                claims,
                verifier: verifier.to_string(),
                redirect_uri: self.redirect_uri.clone(),
                expires_at_secs: chrono::Utc::now().timestamp() + CODE_TTL_SECS,
            },
        );
        code
    }
}

impl Clone for Simulator {
    fn clone(&self) -> Self {
        let key = RsaKeyPair::from_pkcs8(TEST_RSA_PRIVATE_PKCS8_DER).expect("test rsa key loads");
        Self {
            issuer: self.issuer.clone(),
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            kid: self.kid.clone(),
            n: self.n.clone(),
            e: self.e.clone(),
            key,
            codes: Mutex::new(
                self.codes
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            IssuedCode {
                                claims: v.claims.clone(),
                                verifier: v.verifier.clone(),
                                redirect_uri: v.redirect_uri.clone(),
                                expires_at_secs: v.expires_at_secs,
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }
}

#[async_trait::async_trait]
impl OidcTransport for Simulator {
    async fn exchange_code(&self, request: &TokenRequest) -> Result<TokenResponse, TransportError> {
        let mut codes = self.codes.lock().unwrap();
        let issued = codes
            .remove(&request.code)
            .ok_or_else(|| TransportError("unknown auth code".into()))?;
        if issued.redirect_uri != request.redirect_uri {
            return Err(TransportError("redirect_uri mismatch".into()));
        }
        if issued.verifier != request.code_verifier {
            return Err(TransportError("PKCE verifier mismatch".into()));
        }
        if chrono::Utc::now().timestamp() > issued.expires_at_secs {
            return Err(TransportError("auth code expired".into()));
        }
        let id_token = self.sign_id_token(&issued.claims);
        Ok(TokenResponse { id_token })
    }

    async fn fetch_jwks(&self) -> Result<jwks::Jwks, TransportError> {
        Ok(self.jwks())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spki_extraction_roundtrips_with_ring_verify() {
        let key = RsaKeyPair::from_pkcs8(TEST_RSA_PRIVATE_PKCS8_DER).unwrap();
        let (n, e) = spki_rsa_n_e(TEST_RSA_PUBLIC_SPKI_DER).expect("extract");
        let msg = b"roundtrip message";
        let mut sig = vec![0u8; key.public().modulus_len()];
        key.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), msg, &mut sig)
            .unwrap();
        ring::signature::RsaPublicKeyComponents { n: &n, e: &e }
            .verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, msg, &sig)
            .expect("extracted n/e verify a real signature");
    }

    #[test]
    fn signed_token_verifies_against_served_jwks() {
        let sim = Simulator::new("https://issuer", "cid", "https://app/cb");
        let token = sim.sign_id_token(&serde_json::json!({"sub": "s", "exp": 1}));
        let jwks = sim.jwks();
        let jwk = jwks.key_for_kid(&sim.kid).unwrap();
        crate::oidc::jwt::verify_rs256(&token, jwk).expect("jwks key verifies");
    }
}
