//! Actor-scoped HTTP boundary for owner intraday quote demand mutations.
//!
//! The durable mutation algorithm remains in the public job-queue repository.
//! This adapter derives the owner from the authenticated actor and selects the
//! repository's narrow current-identity mutation seam so its locks, replay
//! rules, capacity checks, and tombstones remain single-sourced.

use crate::actor_tx::actor_uuid;
use auth::entitlement::Actor;
use job_queue::owner_equity_v2::{
    DemandMutationOutcome, DemandReleaseOutcome, IntradayCacheRecord, IntradayCalendarReadState,
    IntradayIdentityReadState, IntradayQuoteDemandRequest, IntradayQuoteReleaseRequest,
    IntradaySessionProof, IntradayStorageError, OwnerIntradayQuoteRepository,
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

    /// Read the exact current READY identity for the authenticated owner.
    /// The durable repository owns the RLS transaction and current-admission
    /// semantics; this adapter only derives the owner from the actor.
    pub async fn read_current_identity_state(
        &self,
        actor: &Actor,
        membership_id: Uuid,
        instrument_id: &str,
        generation: u64,
    ) -> Result<Option<IntradayIdentityReadState>, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .read_current_identity_state(owner, membership_id, instrument_id, generation)
            .await
    }

    /// Read the current KIS calendar disposition through the accepted
    /// actor-scoped repository seam.  No provider or credential state is
    /// consulted here.
    pub async fn read_current_calendar_disposition(
        &self,
        actor: &Actor,
    ) -> Result<Option<IntradayCalendarReadState>, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .read_current_calendar_disposition(owner)
            .await
    }

    /// Read the exact current identity/session cache row through the accepted
    /// read-only repository seam.
    pub async fn read_current_cache(
        &self,
        actor: &Actor,
        membership_id: Uuid,
        generation: u64,
        session: &IntradaySessionProof,
    ) -> Result<Option<IntradayCacheRecord>, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .read_current_cache(owner, membership_id, generation, session)
            .await
    }
}

fn owner_uuid(actor: &Actor) -> Result<Uuid, IntradayStorageError> {
    if !actor.is_owner() {
        return Err(IntradayStorageError::PermissionDenied);
    }
    actor_uuid(actor).map_err(|_| IntradayStorageError::PermissionDenied)
}
