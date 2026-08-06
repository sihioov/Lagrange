//! Router state: pools, config, the entitlement service, and the idempotency
//! store. `ApiState` is the `S` for every handler's `State<S>` extractor and
//! implements [`SessionBackend`] so the session extractor can resolve
//! cookies through the admin pool.

use crate::actor_tx::pool_for_actor;
use crate::error::TenancyError;
use crate::http::idempotency::{IdempotencyStore, InMemoryIdempotencyStore};
use crate::repos::accounts::AccountRepo;
use crate::repos::admin::AdminRepo;
use crate::repos::artifacts::ArtifactRepo;
use crate::repos::audit::AuditWriter;
use crate::repos::backtest_runs::BacktestRunRepo;
use crate::repos::entitlements::EntitlementRepo;
use crate::repos::metrics::MetricsRepo;
use crate::repos::ops::OpsRepo;
use crate::repos::paper::PaperRepo;
use crate::repos::recommendations::RecommendationRepo;
use crate::repos::shared::SharedDataRepo;
use crate::repos::strategies::StrategyCatalogRepo;
use crate::repos::strategy_configs::StrategyConfigRepo;
use auth::entitlement::{Actor, EntitlementService};
use job_queue::JobQueue;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Pools the session extractor may use to resolve cookies (the admin role's
/// `USING (true)` SELECT policy on `web_sessions` is the documented path).
pub trait SessionBackend: Send + Sync {
    fn admin_pool(&self) -> &sqlx::PgPool;
    fn app_pool(&self) -> &sqlx::PgPool;
}

/// Runtime configuration of the API surface.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Secret signing pagination cursors (32 bytes).
    pub cursor_secret: [u8; 32],
    /// Per-owner cap on QUEUED+RUNNING jobs (BACKTEST_CAPACITY_EXCEEDED).
    pub max_jobs_per_owner: u32,
    /// App-role database URL; actor-GUC pools for queue calls derive from it.
    pub db_url: String,
    /// Max age of the authentication event behind Owner step-up actions.
    pub step_up_max_auth_age_secs: i64,
}

/// The assembled router state.
#[derive(Clone)]
pub struct ApiState {
    pub cfg: Arc<ApiConfig>,
    pub app_pool: sqlx::PgPool,
    pub admin_pool: sqlx::PgPool,
    pub audit_pool: sqlx::PgPool,
    pub entitlements: Arc<EntitlementService>,
    pub idempotency: Arc<dyn IdempotencyStore>,
    actor_pools: Arc<Mutex<HashMap<String, sqlx::PgPool>>>,
}

impl SessionBackend for ApiState {
    fn admin_pool(&self) -> &sqlx::PgPool {
        &self.admin_pool
    }
    fn app_pool(&self) -> &sqlx::PgPool {
        &self.app_pool
    }
}

impl ApiState {
    /// Build state from the three serving pools, loading the entitlement
    /// service from `data_entitlements` (fail-closed when empty).
    pub async fn from_pools(
        cfg: ApiConfig,
        app_pool: sqlx::PgPool,
        admin_pool: sqlx::PgPool,
        audit_pool: sqlx::PgPool,
    ) -> crate::error::TenancyResult<ApiState> {
        let entitlements = EntitlementRepo::new(app_pool.clone()).load().await?;
        Ok(ApiState {
            cfg: Arc::new(cfg),
            app_pool,
            admin_pool,
            audit_pool,
            entitlements: Arc::new(EntitlementService::new(entitlements)),
            idempotency: Arc::new(InMemoryIdempotencyStore::default()),
            actor_pools: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn strategy_catalog(&self) -> StrategyCatalogRepo {
        StrategyCatalogRepo::new(self.app_pool.clone())
    }
    pub fn strategy_configs(&self) -> StrategyConfigRepo {
        StrategyConfigRepo::new(self.app_pool.clone())
    }
    pub fn accounts(&self) -> AccountRepo {
        AccountRepo::new(self.app_pool.clone())
    }
    pub fn shared(&self) -> SharedDataRepo {
        SharedDataRepo::new(self.app_pool.clone())
    }
    pub fn backtest_runs(&self) -> BacktestRunRepo {
        BacktestRunRepo::new(self.app_pool.clone())
    }
    pub fn artifacts(&self) -> ArtifactRepo {
        ArtifactRepo::new(self.app_pool.clone())
    }
    pub fn recommendations(&self) -> RecommendationRepo {
        RecommendationRepo::new(self.app_pool.clone())
    }
    pub fn metrics(&self) -> MetricsRepo {
        MetricsRepo::new(self.app_pool.clone())
    }
    pub fn paper(&self) -> PaperRepo {
        PaperRepo::new(self.app_pool.clone())
    }
    pub fn ops(&self) -> OpsRepo {
        OpsRepo::new(
            self.admin_pool.clone(),
            self.audit_pool.clone(),
            self.cfg.db_url.clone(),
        )
    }
    pub fn admin(&self) -> AdminRepo {
        AdminRepo::new(
            self.admin_pool.clone(),
            AuditWriter::new(self.audit_pool.clone()),
        )
    }
    pub fn audit_writer(&self) -> AuditWriter {
        AuditWriter::new(self.audit_pool.clone())
    }

    /// A job-queue client whose statements run under the actor's RLS context.
    /// The queue opens its own transactions, so the actor GUC rides in via
    /// per-actor connection pools (the documented T23 P1 wiring); pools are
    /// cached per user id.
    pub async fn queue_for(&self, actor: &Actor) -> crate::error::TenancyResult<JobQueue> {
        let user = actor.user_id.0.clone();
        let pool = match self.actor_pools.lock() {
            Ok(map) => map.get(&user).cloned(),
            Err(_) => None,
        };
        let pool = match pool {
            Some(p) => p,
            None => {
                let p = pool_for_actor(&self.cfg.db_url, &user, 2).await?;
                if let Ok(mut map) = self.actor_pools.lock() {
                    map.insert(user.clone(), p.clone());
                }
                p
            }
        };
        Ok(JobQueue::new(
            pool,
            Some(self.audit_pool.clone()),
            Default::default(),
        ))
    }

    /// Load the job by id through an actor-scoped queue (RLS-scoped read).
    pub async fn queue_get(
        &self,
        actor: &Actor,
        job_id: uuid::Uuid,
    ) -> Result<Option<job_queue::Job>, TenancyError> {
        let queue = self.queue_for(actor).await?;
        match queue.get_by_id(job_id).await {
            Ok(job) => Ok(Some(job)),
            Err(job_queue::QueueError::JobNotFound(_)) => Ok(None),
            Err(job_queue::QueueError::Database(e)) => Err(TenancyError::Database(e)),
            Err(e) => Err(TenancyError::Database(sqlx::Error::Protocol(e.to_string()))),
        }
    }
}
