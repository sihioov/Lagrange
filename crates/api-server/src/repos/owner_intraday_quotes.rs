//! Actor-scoped HTTP boundary for owner intraday quote demand mutations.
//!
//! The durable mutation algorithm remains in the public job-queue repository.
//! This adapter derives the owner from the authenticated actor and selects the
//! repository's narrow current-identity mutation seam so its locks, replay
//! rules, capacity checks, and tombstones remain single-sourced.

use crate::actor_tx::actor_uuid;
use auth::entitlement::Actor;
use job_queue::owner_equity_v2::{
    DemandMutationOutcome, DemandReleaseOutcome, IntradayQuoteDemandRequest,
    IntradayQuoteReleaseRequest, IntradayStorageError, OwnerIntradayQuoteRepository,
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct OwnerIntradayQuoteRepo {
    pool: PgPool,
}

impl OwnerIntradayQuoteRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Delegate to the durable repository's same-transaction current-identity
    /// mutation seam, which makes the authoritative check after its own locks.
    pub async fn create_or_renew_demand(
        &self,
        actor: &Actor,
        request: &IntradayQuoteDemandRequest,
    ) -> Result<DemandMutationOutcome, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .create_or_renew_demand_current(owner, request)
            .await
    }

    /// Release through the durable repository's current-identity HTTP seam.
    pub async fn release_demand(
        &self,
        actor: &Actor,
        demand_id: Uuid,
        request: &IntradayQuoteReleaseRequest,
    ) -> Result<DemandReleaseOutcome, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .release_demand_current(owner, demand_id, request)
            .await
    }
}

fn owner_uuid(actor: &Actor) -> Result<Uuid, IntradayStorageError> {
    if !actor.is_owner() {
        return Err(IntradayStorageError::PermissionDenied);
    }
    actor_uuid(actor).map_err(|_| IntradayStorageError::PermissionDenied)
}
