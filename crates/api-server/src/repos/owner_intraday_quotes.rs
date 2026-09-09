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

    /// Observe the actor-visible current admission before delegating the
    /// mutation.  The durable repository repeats this check after its own
    /// owner lock; the observation here is intentionally non-authoritative so
    /// an exact durable replay can still succeed after a later disable or
    /// generation change.
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
