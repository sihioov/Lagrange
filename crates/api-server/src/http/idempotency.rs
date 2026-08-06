//! HTTP-layer idempotency (FR-BT-008 / design §12.1): mutating routes carry
//! an `Idempotency-Key`; the first request stores key -> (body hash, cached
//! response); a replay with the same body returns the cached result (same
//! side effect), a replay with a different body is a typed
//! `IDEMPOTENCY_KEY_MISMATCH`. Job-creating routes additionally pass the key
//! down to the queue's per-owner `jobs.idempotency_key` (DB-level dedup).

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// One cached idempotent result (bounded; single-instance API server).
#[derive(Debug, Clone)]
pub struct CachedResult {
    pub body_hash: String,
    pub status: StatusCode,
    pub body: Value,
}

/// The idempotency store contract (in-memory today; a DB impl can swap in
/// without touching handlers).
pub trait IdempotencyStore: Send + Sync {
    fn get(&self, key: &str) -> Option<CachedResult>;
    fn insert(&self, key: &str, cached: CachedResult);
}

/// Bounded in-memory store, keyed by `{actor_user_id}:{key}`.
#[derive(Debug, Default)]
pub struct InMemoryIdempotencyStore {
    inner: Mutex<HashMap<String, CachedResult>>,
}

const MAX_ENTRIES: usize = 4096;

impl IdempotencyStore for InMemoryIdempotencyStore {
    fn get(&self, key: &str) -> Option<CachedResult> {
        self.inner.lock().ok()?.get(key).cloned()
    }

    fn insert(&self, key: &str, cached: CachedResult) {
        if let Ok(mut map) = self.inner.lock() {
            if map.len() >= MAX_ENTRIES && !map.contains_key(key) {
                map.clear();
            }
            map.insert(key.to_string(), cached);
        }
    }
}

/// The `Idempotency-Key` header name.
pub const HEADER: &str = "idempotency-key";

/// Read the Idempotency-Key header value (trimmed), if present.
pub fn key_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Canonical hash of the request body for replay comparison.
pub fn body_hash(body: &Value) -> String {
    crate::http::validation::sha256_hex(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip_and_key_isolation() {
        let store = InMemoryIdempotencyStore::default();
        assert!(store.get("a:1").is_none());
        store.insert(
            "a:1",
            CachedResult {
                body_hash: "h".into(),
                status: StatusCode::CREATED,
                body: serde_json::json!({"id": "x"}),
            },
        );
        let cached = store.get("a:1").expect("cached");
        assert_eq!(cached.body["id"], "x");
        assert_eq!(cached.status, StatusCode::CREATED);
        // Different actor scope is isolated.
        assert!(store.get("b:1").is_none());
    }
}
