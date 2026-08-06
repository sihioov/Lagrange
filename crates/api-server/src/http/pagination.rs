//! Opaque cursor pagination (design §12.1): base64url(HMAC-signed payload)
//! where the payload is `{k: <sort key>, i: <id>}`. Cursors are unforgeable
//! (server secret), stable under a fixed ordering, and decode failures are
//! typed [`CursorError::Invalid`] -> `INVALID_CURSOR`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The keyset position a cursor points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// RFC3339 sort key of the anchor row (created_at).
    pub k: String,
    /// Row id (uuid) for the tie-break.
    pub i: String,
}

impl Cursor {
    /// Encode + sign. The signature covers the payload, so tampering or
    /// forging a cursor fails `Invalid`.
    pub fn encode(&self, secret: &[u8]) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor json"));
        let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
        mac.update(payload.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{payload}.{sig}")
    }

    /// Decode + verify. Rejects malformed, tampered, and forged cursors.
    pub fn decode(raw: &str, secret: &[u8]) -> Result<Cursor, CursorError> {
        let (payload, sig) = raw.split_once('.').ok_or(CursorError::Invalid)?;
        let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
        mac.update(payload.as_bytes());
        mac.verify_slice(
            &URL_SAFE_NO_PAD
                .decode(sig)
                .map_err(|_| CursorError::Invalid)?,
        )
        .map_err(|_| CursorError::Invalid)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| CursorError::Invalid)?;
        serde_json::from_slice(&bytes).map_err(|_| CursorError::Invalid)
    }
}

/// Cursor failures (all map to `INVALID_CURSOR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorError {
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8; 32] = b"cursor-test-secret-0123456789abc";

    #[test]
    fn cursor_roundtrip() {
        let c = Cursor {
            k: "2026-01-30T00:00:00Z".into(),
            i: "11111111-2222-3333-4444-555555555555".into(),
        };
        let enc = c.encode(SECRET);
        assert_eq!(Cursor::decode(&enc, SECRET).unwrap(), c);
    }

    #[test]
    fn cursor_tamper_and_forge_rejected() {
        let c = Cursor {
            k: "2026-01-30T00:00:00Z".into(),
            i: "11111111-2222-3333-4444-555555555555".into(),
        };
        let enc = c.encode(SECRET);
        // Flip one payload char -> signature mismatch.
        let mut tampered = enc.clone();
        let first = tampered.as_bytes()[0];
        let replacement = if first == b'A' { b'B' } else { b'A' };
        tampered.replace_range(0..1, &(replacement as char).to_string());
        assert_eq!(Cursor::decode(&tampered, SECRET), Err(CursorError::Invalid));
        // Wrong secret -> Invalid.
        assert_eq!(
            Cursor::decode(&enc, b"other-secret-0123456789abcdef____"),
            Err(CursorError::Invalid)
        );
        // Garbage -> Invalid.
        assert_eq!(Cursor::decode("garbage", SECRET), Err(CursorError::Invalid));
        // Missing signature -> Invalid.
        assert_eq!(
            Cursor::decode("only-payload", SECRET),
            Err(CursorError::Invalid)
        );
    }

    #[test]
    fn cursor_encodes_to_opaque_base64() {
        let c = Cursor {
            k: "2026-01-30T00:00:00Z".into(),
            i: "11111111-2222-3333-4444-555555555555".into(),
        };
        let enc = c.encode(SECRET);
        assert!(enc.contains('.'), "payload.signature form");
        assert!(!enc.contains(':'), "no raw timestamp in the wire form");
        assert!(enc.len() < 200);
    }
}
