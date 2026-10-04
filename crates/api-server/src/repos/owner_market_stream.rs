//! Actor/session-bound adapter. Durable lease mutation and snapshot rules
//! remain in the job-queue repository; this layer attests deployment pins.

use auth::entitlement::Actor;
use job_queue::owner_equity_v2::{
    MarketStreamStorageError as Error, OwnerMarketStreamRepository, ReleaseOutcome, StreamLease,
    StreamLeaseRequest, StreamSnapshot,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::http::owner_market_stream_config::MarketStreamReadPins;

#[derive(Clone)]
pub(crate) struct OwnerMarketStreamRepo {
    pool: PgPool,
    pins: MarketStreamReadPins,
}

impl OwnerMarketStreamRepo {
    pub(crate) fn new(pool: PgPool, pins: MarketStreamReadPins) -> Self {
        Self { pool, pins }
    }

    /// An app-only boolean capability checks the configured slot/grant/source
    /// contract without giving the API SELECT access to the private grant.
    pub(crate) async fn check_binding(
        &self,
        actor: &Actor,
        session_hash: &str,
    ) -> Result<Uuid, Error> {
        if !actor.is_owner() {
            return Err(Error::SessionInvalid);
        }
        let owner = actor_uuid(actor).map_err(|_| Error::SessionInvalid)?;
        let mut tx = begin_actor_tx(&self.pool, actor)
            .await
            .map_err(|_| Error::DatabaseIntegrity)?;
        sqlx::query("SET LOCAL statement_timeout = '1s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::DatabaseIntegrity)?;
        sqlx::query("SET LOCAL lock_timeout = '1s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::DatabaseIntegrity)?;
        sqlx::query("SELECT pg_catalog.set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::DatabaseIntegrity)?;
        let valid: bool = sqlx::query_scalar(
            "SELECT public.owner_market_stream_api_binding_valid($1, $2, $3, $4, $5)",
        )
        .bind(owner)
        .bind(session_hash)
        .bind(self.pins.credential_slot_id)
        .bind(self.pins.grant_id)
        .bind(&self.pins.contract_sha256)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| Error::DatabaseIntegrity)?;
        tx.commit().await.map_err(|_| Error::DatabaseIntegrity)?;
        if !valid {
            return Err(Error::RightsInvalid);
        }
        Ok(owner)
    }

    pub(crate) async fn create_or_renew(
        &self,
        actor: &Actor,
        session_hash: &str,
        request: &StreamLeaseRequest,
    ) -> Result<StreamLease, Error> {
        let owner = self.check_binding(actor, session_hash).await?;
        OwnerMarketStreamRepository::new(self.pool.clone())
            .replace_stream_lease(owner, session_hash, request)
            .await
    }

    pub(crate) async fn release(
        &self,
        actor: &Actor,
        session_hash: &str,
        lease: Uuid,
        consumer: Uuid,
        sequence: u64,
    ) -> Result<ReleaseOutcome, Error> {
        let owner = self.check_binding(actor, session_hash).await?;
        OwnerMarketStreamRepository::new(self.pool.clone())
            .release_stream_lease_for_consumer(owner, session_hash, lease, consumer, sequence)
            .await
    }

    pub(crate) async fn snapshot(
        &self,
        actor: &Actor,
        session_hash: &str,
        lease: Uuid,
    ) -> Result<StreamSnapshot, Error> {
        let owner = self.check_binding(actor, session_hash).await?;
        OwnerMarketStreamRepository::new(self.pool.clone())
            .read_stream_snapshot_with_window(
                owner,
                session_hash,
                lease,
                self.pins.window.as_deref(),
            )
            .await
    }
}
