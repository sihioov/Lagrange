//! Actor-scoped HTTP boundary for owner intraday quote demand mutations.
//!
//! The durable mutation algorithm remains in the public job-queue repository.
//! This adapter derives the owner from the authenticated actor, observes the
//! current 0053 admission through the narrow read seam, and then delegates the
//! mutation so its locks, replay rules, capacity checks, and tombstones remain
//! single-sourced.

use auth::entitlement::Actor;
use job_queue::owner_equity_v2::{
    DemandMutationOutcome, DemandReleaseOutcome, IntradayQuoteDemandRequest,
    IntradayQuoteReleaseRequest, IntradayStorageError, OwnerIntradayQuoteRepository,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::actor_tx::{actor_uuid, begin_actor_tx};

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
        self.observe_current_admission(actor, owner, request.membership_id, request.generation)
            .await?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .create_or_renew_demand(owner, request)
            .await
    }

    /// Delegate release directly to the durable repository.  In particular,
    /// replaying a committed release does not require the membership to remain
    /// READY: RELEASED is a terminal tombstone owned by the demand repository.
    pub async fn release_demand(
        &self,
        actor: &Actor,
        demand_id: Uuid,
        request: &IntradayQuoteReleaseRequest,
    ) -> Result<DemandReleaseOutcome, IntradayStorageError> {
        let owner = owner_uuid(actor)?;
        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .release_demand(owner, demand_id, request)
            .await
    }

    async fn observe_current_admission(
        &self,
        actor: &Actor,
        owner: Uuid,
        membership_id: Uuid,
        generation: u64,
    ) -> Result<(), IntradayStorageError> {
        let instrument_id = self.membership_in(actor, membership_id).await?;
        let Some(instrument_id) = instrument_id else {
            // The mutation repository remains authoritative for the final
            // not-found/privacy result and for durable replay precedence.
            return Ok(());
        };

        OwnerIntradayQuoteRepository::new(self.pool.clone())
            .read_current_identity_state(owner, membership_id, &instrument_id, generation)
            .await
            .map(|_| ())
            .map_err(|error| match error {
                // A malformed persisted identity is storage corruption, not a
                // caller parameter error.
                IntradayStorageError::InvalidInput => IntradayStorageError::DatabaseIntegrity,
                other => other,
            })
    }

    async fn membership_in(
        &self,
        actor: &Actor,
        membership_id: Uuid,
    ) -> Result<Option<String>, IntradayStorageError> {
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(map_tenancy_error)?;
        let instrument_id: Option<String> = sqlx::query_scalar(
            "SELECT instrument_id
               FROM public.owner_equity_memberships
              WHERE id = $1",
        )
        .bind(membership_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database_error)?;
        tx.commit().await.map_err(map_database_error)?;
        Ok(instrument_id)
    }
}

fn owner_uuid(actor: &Actor) -> Result<Uuid, IntradayStorageError> {
    if !actor.is_owner() {
        return Err(IntradayStorageError::PermissionDenied);
    }
    actor_uuid(actor).map_err(|_| IntradayStorageError::PermissionDenied)
}

fn map_tenancy_error(error: crate::error::TenancyError) -> IntradayStorageError {
    match error {
        crate::error::TenancyError::Forbidden => IntradayStorageError::PermissionDenied,
        crate::error::TenancyError::NotFound => IntradayStorageError::DemandNotFound,
        crate::error::TenancyError::Database(error) => map_database_error(error),
        _ => IntradayStorageError::DatabaseIntegrity,
    }
}

fn map_database_error(error: sqlx::Error) -> IntradayStorageError {
    if matches!(&error, sqlx::Error::Database(database) if database.code().as_deref() == Some("42501"))
    {
        IntradayStorageError::PermissionDenied
    } else {
        IntradayStorageError::DatabaseUnavailable
    }
}
