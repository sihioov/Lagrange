//! HTTP-layer idempotency (FR-BT-008 / design §12.1): mutating routes carry
//! an `Idempotency-Key`; the first request stores key -> (body hash, cached
//! response); a replay with the same body returns the cached result (same
//! side effect), a replay with a different body is a typed
//! `IDEMPOTENCY_KEY_MISMATCH`. Job-creating routes additionally pass the key
//! down to the queue's per-owner `jobs.idempotency_key` (DB-level dedup).

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    fn gate(&self, key: &str) -> Arc<tokio::sync::Mutex<()>>;
}

/// Bounded in-memory store, keyed by `{actor_user_id}:{key}`.
#[derive(Debug, Default)]
pub struct InMemoryIdempotencyStore {
    inner: Mutex<HashMap<String, CachedResult>>,
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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
            map.entry(key.to_string()).or_insert(cached);
        }
    }

    fn gate(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.gates.lock().expect("idempotency gate mutex poisoned");
        if gates.len() >= MAX_ENTRIES && !gates.contains_key(key) {
            // An idle gate has only the map's Arc. Never evict a gate with a
            // holder or waiter, because that would split one key into two
            // concurrent flights.
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        }
        gates
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

/// The `Idempotency-Key` header name.
pub const HEADER: &str = "idempotency-key";
/// Leaves room for server-owned queue namespaces while bounding cache and DB
/// identity amplification from a public header.
pub const MAX_KEY_BYTES: usize = 200;

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

    #[test]
    fn committed_result_is_first_write_wins() {
        let store = InMemoryIdempotencyStore::default();
        store.insert(
            "a:race",
            CachedResult {
                body_hash: "public-body".into(),
                status: StatusCode::CREATED,
                body: serde_json::json!({"id": "committed-run"}),
            },
        );
        store.insert(
            "a:race",
            CachedResult {
                body_hash: "different-body".into(),
                status: StatusCode::CONFLICT,
                body: serde_json::json!({"error": {"code": "IDEMPOTENCY_KEY_MISMATCH"}}),
            },
        );

        let cached = store.get("a:race").expect("first result remains cached");
        assert_eq!(cached.status, StatusCode::CREATED);
        assert_eq!(cached.body["id"], "committed-run");
    }

    #[tokio::test]
    async fn same_key_gate_allows_only_one_in_flight_request() {
        let store = InMemoryIdempotencyStore::default();
        let gate = store.gate("a:race");
        let first = gate.clone().lock_owned().await;
        let waiting = gate.clone().try_lock_owned();
        assert!(waiting.is_err(), "a second request must wait for the first");
        drop(first);
        assert!(gate.try_lock_owned().is_ok());
    }
}
