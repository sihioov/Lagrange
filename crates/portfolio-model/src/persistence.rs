//! Persistence seam — DB manifests land with Todo 3.
//!
//! The ledger core is deliberately DB-free (no sqlx, no `migrations/`
//! dependency): acceptance is pure in-memory invariants. This module is the
//! contract a Todo-3 PostgreSQL-backed store will implement, mirroring the
//! accepted `SessionStore` seam precedent from Todo 22. Today,
//! [`InMemoryLedgerStore`] serves backtest/Paper and the replay suites;
//! Paper and Live snapshots move to PostgreSQL manifests when Todo 3 lands.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use crate::error::PortfolioError;
use crate::ledger::LedgerState;

/// The ledger persistence contract (DB implementation lands with Todo 3).
pub trait LedgerStore {
    /// Persists a full ledger snapshot for an account.
    fn save_snapshot(&self, account_id: &str, state: &LedgerState) -> Result<(), PortfolioError>;
    /// Loads the account snapshot (None when absent).
    fn load_snapshot(&self, account_id: &str) -> Result<Option<LedgerState>, PortfolioError>;
}

/// The in-memory implementation used until Todo 3 provides PostgreSQL.
#[derive(Debug, Default)]
pub struct InMemoryLedgerStore {
    snapshots: Mutex<BTreeMap<String, LedgerState>>,
}

impl InMemoryLedgerStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl LedgerStore for InMemoryLedgerStore {
    fn save_snapshot(&self, account_id: &str, state: &LedgerState) -> Result<(), PortfolioError> {
        let mut snapshots = self.lock()?;
        snapshots.insert(account_id.to_owned(), state.clone());
        Ok(())
    }

    fn load_snapshot(&self, account_id: &str) -> Result<Option<LedgerState>, PortfolioError> {
        let snapshots = self.lock()?;
        Ok(snapshots.get(account_id).cloned())
    }
}

impl InMemoryLedgerStore {
    fn lock(&self) -> Result<MutexGuard<'_, BTreeMap<String, LedgerState>>, PortfolioError> {
        self.snapshots
            .lock()
            .map_err(|_| PortfolioError::Serialization {
                detail: "in-memory ledger store lock poisoned".to_owned(),
            })
    }
}
