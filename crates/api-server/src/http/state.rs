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
use crate::repos::parity::ParityRepo;
use crate::repos::pending_targets::PendingTargetRepo;
use crate::repos::rebalance_previews::RebalancePreviewRepo;
use crate::repos::recommendations::RecommendationRepo;
use crate::repos::robustness::RobustnessRepo;
use crate::repos::shared::SharedDataRepo;
use crate::repos::strategies::StrategyCatalogRepo;
use crate::repos::strategy_configs::StrategyConfigRepo;
use auth::entitlement::{Actor, EntitlementService};
use job_queue::JobQueue;
use job_queue::recommendation::input::DatasetPin;
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
    /// Immutable dataset identity selected by deployment configuration for
    /// every manual recommendation. Submission re-attests this exact pin
    /// against `dataset_versions`; it never selects a newer READY version.
    pub recommendation_dataset: DatasetPin,
    /// App-role database URL; actor-GUC pools for queue calls derive from it.
    pub db_url: String,
    /// Max age of the authentication event behind Owner step-up actions.
    pub step_up_max_auth_age_secs: i64,
    /// Read-only artifact tree (mirrors the `/data/artifacts` mount); the
    /// download route hashes files against the manifest before serving.
    pub artifact_root: std::path::PathBuf,
    /// Seoul civil date used by date-sensitive submission/apply gates.
    /// Production supplies [`system_seoul_today`]; tests can pin a date so
    /// calendar fixtures do not expire as wall time advances.
    pub seoul_today: fn() -> chrono::NaiveDate,
    /// Immutable API image revision copied to newly-created backtest runs.
    pub code_commit: String,
}

pub fn system_seoul_today() -> chrono::NaiveDate {
    let offset = chrono::FixedOffset::east_opt(9 * 60 * 60).expect("fixed Seoul offset");
    chrono::Utc::now().with_timezone(&offset).date_naive()
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

    /// Check every role-scoped pool used by the HTTP process.
    ///
    /// Readiness is deliberately stricter than liveness: an API process is
    /// only ready when the app, admin, and append-only audit connections can
    /// all complete a round trip.  Keeping this check on the state rather
    /// than reaching into individual handlers also makes it impossible for a
    /// deployment probe to accidentally exercise an RLS-sensitive endpoint.
    pub async fn check_readiness(&self) -> Result<(), sqlx::Error> {
        for pool in [&self.app_pool, &self.admin_pool, &self.audit_pool] {
            sqlx::query("SELECT 1").execute(pool).await?;
        }
        Ok(())
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
    pub fn robustness_suites(&self) -> RobustnessRepo {
        RobustnessRepo::new(self.app_pool.clone())
    }
    pub fn artifacts(&self) -> ArtifactRepo {
        ArtifactRepo::new(self.app_pool.clone())
    }
    pub fn recommendations(&self) -> RecommendationRepo {
        RecommendationRepo::new(self.app_pool.clone())
    }
    pub fn rebalance_previews(&self) -> RebalancePreviewRepo {
        RebalancePreviewRepo::new(self.app_pool.clone())
    }
    /// The Live repository, bound to the calling actor.
    ///
    /// Takes the actor rather than exposing a bare pool: `broker_connections`
    /// is a FORCE-RLS tenant table, so a query without the actor GUC is
    /// refused. Requiring it here means a handler cannot reach these rows
    /// without having established who is asking.
    pub fn live(&self, actor: &auth::entitlement::Actor) -> crate::repos::live::LiveRepo {
        crate::repos::live::LiveRepo::new(self.app_pool.clone(), actor.clone())
    }
    /// Live order intents for one actor.
    pub fn order_intents(
        &self,
        actor: &auth::entitlement::Actor,
        owner_user_id: uuid::Uuid,
    ) -> crate::repos::order_intents::OrderIntentRepo {
        crate::repos::order_intents::OrderIntentRepo::new(
            self.app_pool.clone(),
            actor.clone(),
            owner_user_id,
        )
    }

    /// The Risk Gateway's decision store for one actor.
    pub fn risk(
        &self,
        actor: &auth::entitlement::Actor,
        owner_user_id: uuid::Uuid,
        account_id: Option<uuid::Uuid>,
    ) -> crate::repos::risk::RiskRepo {
        crate::repos::risk::RiskRepo::new(
            self.app_pool.clone(),
            actor.clone(),
            owner_user_id,
            account_id,
        )
    }

    /// Reconciliation runs and readiness for one actor.
    ///
    /// Takes the owner id explicitly rather than parsing it out of the actor,
    /// because a non-uuid actor id cannot address a tenant row at all and the
    /// caller has already resolved the Owner by the time it asks.
    pub fn reconciliation(
        &self,
        actor: &auth::entitlement::Actor,
        owner_user_id: uuid::Uuid,
    ) -> crate::repos::reconciliation::ReconciliationRepo {
        crate::repos::reconciliation::ReconciliationRepo::new(
            self.app_pool.clone(),
            actor.clone(),
            owner_user_id,
        )
    }
    pub fn metrics(&self) -> MetricsRepo {
        MetricsRepo::new(self.app_pool.clone())
    }
    pub fn paper(&self) -> PaperRepo {
        PaperRepo::new(self.app_pool.clone())
    }
    pub fn pending_targets(&self) -> PendingTargetRepo {
        PendingTargetRepo::new(self.app_pool.clone())
    }
    pub fn parity(&self) -> ParityRepo {
        ParityRepo::new(self.app_pool.clone())
    }

    /// The backtest-vs-Paper parity report for one account and session,
    /// computed on read (never stored, so it cannot go stale).
    pub async fn parity_report(
        &self,
        actor: &Actor,
        account_id: uuid::Uuid,
        as_of: &str,
    ) -> crate::error::TenancyResult<result_model::paper_parity::ParityReport> {
        self.parity().report(actor, account_id, as_of).await
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

    pub fn notifier(&self) -> crate::notify::Notifier {
        crate::notify::Notifier::new(self.app_pool.clone(), self.admin_pool.clone())
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
