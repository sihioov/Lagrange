//! Private, one-use startup binding for the official simulation process.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use market_data::{ApprovedHistoricalPriceOnlyArtifact, approve_historical_price_only_artifact};
use sqlx::{
    PgConnection, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::{sync::watch, time::Instant};

use super::{
    SimulationProcessConfig, SimulationProcessObservation, TrustedSimulationRuntime,
    observe_current_simulation_process,
    process_acl::{
        SimulationWorkerAclError, SimulationWorkerAclObservation, observe_simulation_worker_acl,
    },
    process_configured_input_rights::{
        SimulationConfiguredInputRightsError, SimulationConfiguredInputRightsObservation,
        observe_simulation_configured_input_rights,
    },
    process_migrations::{
        SimulationMigrationLedgerError, SimulationMigrationLedgerObservation,
        observe_simulation_migration_ledger,
    },
    process_rls_metadata::{
        SimulationRlsMetadataError, SimulationRlsMetadataObservation,
        observe_simulation_rls_metadata,
    },
    process_routine_metadata::{
        SimulationVersionHelperMetadataError, SimulationVersionHelperMetadataObservation,
        observe_simulation_version_helper_metadata,
    },
    process_trigger_metadata::{
        SimulationTriggerMetadataError, SimulationTriggerMetadataObservation,
        observe_simulation_trigger_metadata,
    },
    read_simulation_process_config,
};
use crate::{JobQueue, QueueConfig};

/// Constructed only after each observer and recovery has completed. It is
/// consumed by the parent and is neither clonable nor externally constructible.
pub(super) struct StartupEvidence {
    pub(super) config: SimulationProcessConfig,
    pub(super) runtime: TrustedSimulationRuntime,
    pub(super) process: SimulationProcessObservation,
    pub(super) pool: PgPool,
    pub(super) queue: JobQueue,
    pub(super) deadline: Instant,
    pub(super) started_boot_ms: u64,
    pub(super) limit_ms: u64,
    _approval: ApprovedHistoricalPriceOnlyArtifact,
    _migration: SimulationMigrationLedgerObservation,
    _acl: SimulationWorkerAclObservation,
    _routine: SimulationVersionHelperMetadataObservation,
    _rls: SimulationRlsMetadataObservation,
    _trigger: SimulationTriggerMetadataObservation,
    _rights: SimulationConfiguredInputRightsObservation,
}

#[derive(Debug, PartialEq, Eq)]
struct StepFailure<E> {
    operation: Option<E>,
    canceled: bool,
    timed_out: bool,
}

#[derive(Debug)]
pub(super) enum OperationFailure {
    InitialLoad,
    Reader,
    Migration(SimulationMigrationLedgerError),
    Acl(SimulationWorkerAclError),
    Routine(SimulationVersionHelperMetadataError),
    Rls(SimulationRlsMetadataError),
    Trigger(SimulationTriggerMetadataError),
    Rights(SimulationConfiguredInputRightsError),
    Recovery(super::SimulationPublicationError),
}

#[derive(Debug)]
pub(super) enum StartupFailure {
    Rejected,
    Step {
        operation: Option<OperationFailure>,
        canceled: bool,
        timed_out: bool,
    },
}

impl From<()> for StartupFailure {
    fn from(_: ()) -> Self {
        Self::Rejected
    }
}

impl From<StepFailure<OperationFailure>> for StartupFailure {
    fn from(error: StepFailure<OperationFailure>) -> Self {
        Self::Step {
            operation: error.operation,
            canceled: error.canceled,
            timed_out: error.timed_out,
        }
    }
}

impl StartupFailure {
    /// Retain each observer's value-free error and the independent interruption
    /// flags until the fixed-diagnostic CLI boundary consumes the failure.
    pub(super) fn consume(self) {
        if let Self::Step {
            operation,
            canceled,
            timed_out,
        } = self
        {
            let _ = (canceled, timed_out);
            if let Some(operation) = operation {
                match operation {
                    OperationFailure::InitialLoad | OperationFailure::Reader => {}
                    OperationFailure::Migration(error) => {
                        let _ = error;
                    }
                    OperationFailure::Acl(error) => {
                        let _ = error;
                    }
                    OperationFailure::Routine(error) => {
                        let _ = error;
                    }
                    OperationFailure::Rls(error) => {
                        let _ = error;
                    }
                    OperationFailure::Trigger(error) => {
                        let _ = error;
                    }
                    OperationFailure::Rights(error) => {
                        let _ = error;
                    }
                    OperationFailure::Recovery(error) => {
                        let _ = error;
                    }
                }
            }
        }
    }
}

async fn owned_step<F, T, E>(
    future: F,
    deadline: Instant,
    shutdown: &mut watch::Receiver<bool>,
    canceled: &AtomicBool,
) -> Result<T, StepFailure<E>>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    tokio::pin!(future);
    let is_canceled = *shutdown.borrow() || shutdown.has_changed().is_err();
    let timed_out = Instant::now() >= deadline;
    if is_canceled || timed_out {
        canceled.store(true, Ordering::SeqCst);
        return Err(StepFailure {
            operation: None,
            canceled: is_canceled,
            timed_out,
        });
    }
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => {
                canceled.store(true, Ordering::SeqCst);
                let result = future.await;
                return Err(StepFailure { operation: result.err(), canceled: *shutdown.borrow() || shutdown.has_changed().is_err(), timed_out: true });
            }
            change = shutdown.changed() => {
                if change.is_err() || *shutdown.borrow() {
                    canceled.store(true, Ordering::SeqCst);
                    let result = future.await;
                    return Err(StepFailure { operation: result.err(), canceled: true, timed_out: Instant::now() >= deadline });
                }
            }
            result = &mut future => {
                let is_canceled = *shutdown.borrow() || shutdown.has_changed().is_err();
                let timed_out = Instant::now() >= deadline;
                if is_canceled || timed_out {
                    canceled.store(true, Ordering::SeqCst);
                    return Err(StepFailure { operation: result.err(), canceled: is_canceled, timed_out });
                }
                return result.map_err(|error| StepFailure { operation: Some(error), canceled: false, timed_out: false });
            }
        }
    }
}

fn generation_check(
    process: &mut SimulationProcessObservation,
    started: u64,
    limit_ms: u64,
) -> Result<(), ()> {
    let now = process.revalidate().map_err(|_| ())?;
    if now
        .checked_sub(started)
        .filter(|elapsed| *elapsed < limit_ms)
        .is_none()
    {
        return Err(());
    }
    Ok(())
}

fn check_receipt_pin(checked_receipt: &str, expected_bare: &str) -> Result<(), ()> {
    if checked_receipt == format!("sha256:{expected_bare}") {
        Ok(())
    } else {
        Err(())
    }
}

#[allow(clippy::too_many_arguments)] // Match each observer's identity axis independently.
fn identities_match(
    config: &str,
    db: u32,
    worker: u32,
    schema: u32,
    owner: u32,
    hashes: [&str; 6],
    databases: [u32; 5],
    workers: [u32; 5],
    schemas: [u32; 4],
    owners: [u32; 3],
) -> bool {
    hashes.iter().all(|value| *value == config)
        && databases.iter().all(|value| *value == db)
        && workers.iter().all(|value| *value == worker)
        && schemas.iter().all(|value| *value == schema)
        && owners.iter().all(|value| *value == owner)
}

pub(super) async fn startup(
    commit: String,
    config_pin: String,
    receipt_pin: String,
    options: PgConnectOptions,
    mut shutdown: watch::Receiver<bool>,
) -> Result<StartupEvidence, StartupFailure> {
    if *shutdown.borrow() || shutdown.has_changed().is_err() {
        return Err(StartupFailure::Rejected);
    }
    let mut process = observe_current_simulation_process().map_err(|_| ())?;
    let started = process.observed_boot_ms();
    let generation = process.generation();
    // The initial checked config supplies the deadline; the retained BOOTTIME
    // observation includes the time spent reading and checking that config.
    let mut load = tokio::task::spawn_blocking(move || {
        let config = read_simulation_process_config(&commit, &config_pin).map_err(|_| ())?;
        check_receipt_pin(config.receipt_sha256(), &receipt_pin)?;
        let runtime =
            TrustedSimulationRuntime::load_official_config(&config, &commit, &receipt_pin)
                .map_err(|_| ())?;
        runtime.revalidate().map_err(|_| ())?;
        Ok::<_, ()>((config, runtime))
    });
    let loaded = tokio::select! {
        biased;
        change = shutdown.changed() => {
            let _ = change;
            let result = load.await;
            let operation = if matches!(result, Ok(Ok(_))) { None } else { Some(OperationFailure::InitialLoad) };
            return Err(StartupFailure::Step { operation, canceled: true, timed_out: false });
        }
        result = &mut load => result.map_err(|_| ())?,
    }?;
    let (config, runtime) = loaded;
    let limit_ms = config
        .limits()
        .startup_timeout_seconds()
        .checked_mul(1000)
        .ok_or(())?;
    generation_check(&mut process, started, limit_ms)?;
    if process.generation() != generation || *shutdown.borrow() || shutdown.has_changed().is_err() {
        return Err(StartupFailure::Rejected);
    }
    #[cfg(target_os = "linux")]
    let identity = unsafe { (libc::geteuid(), libc::getegid()) };
    #[cfg(not(target_os = "linux"))]
    let identity = (0, 0);
    if identity != (config.reader_uid(), config.reader_gid()) {
        return Err(StartupFailure::Rejected);
    }
    let elapsed = process.observed_boot_ms().checked_sub(started).ok_or(())?;
    let deadline = Instant::now() + Duration::from_millis(limit_ms.checked_sub(elapsed).ok_or(())?);
    let canceled = AtomicBool::new(false);
    let root = PathBuf::from(config.artifact_root());
    let approved = owned_step(
        async move {
            tokio::task::spawn_blocking(move || approve_historical_price_only_artifact(&root))
                .await
                .map_err(|_| OperationFailure::Reader)?
                .map_err(|_| OperationFailure::Reader)
        },
        deadline,
        &mut shutdown,
        &canceled,
    )
    .await?;
    if approved.v3_approval_summary().is_none() {
        return Err(StartupFailure::Rejected);
    }
    generation_check(&mut process, started, limit_ms)?;
    let observers = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy_with(options.clone());
    let result = owned_step(
        async {
            let migration = observe_simulation_migration_ledger(&observers, &config, &canceled)
                .await
                .map_err(OperationFailure::Migration)?;
            let acl = observe_simulation_worker_acl(&observers, &config, &canceled)
                .await
                .map_err(OperationFailure::Acl)?;
            let routine =
                observe_simulation_version_helper_metadata(&observers, &config, &canceled)
                    .await
                    .map_err(OperationFailure::Routine)?;
            let rls = observe_simulation_rls_metadata(&observers, &config, &canceled)
                .await
                .map_err(OperationFailure::Rls)?;
            let trigger = observe_simulation_trigger_metadata(&observers, &config, &canceled)
                .await
                .map_err(OperationFailure::Trigger)?;
            let rights = observe_simulation_configured_input_rights(
                &observers, &config, &approved, &canceled,
            )
            .await
            .map_err(OperationFailure::Rights)?;
            Ok((migration, acl, routine, rls, trigger, rights))
        },
        deadline,
        &mut shutdown,
        &canceled,
    )
    .await;
    observers.close().await;
    let (migration, acl, routine, rls, trigger, rights) = result?;
    generation_check(&mut process, started, limit_ms)?;
    let config_hash = config.config_sha256();
    let db = migration.database_oid();
    let worker = migration.worker_oid();
    let schema = migration.public_schema_oid();
    let owner = migration.migration_owner_oid();
    if !identities_match(
        config_hash,
        db,
        worker,
        schema,
        owner,
        [
            migration.config_sha256(),
            acl.config_sha256(),
            routine.config_sha256(),
            rls.config_sha256(),
            trigger.config_sha256(),
            rights.config_sha256(),
        ],
        [
            acl.database_oid(),
            routine.database_oid(),
            rls.database_oid(),
            trigger.database_oid(),
            rights.database_oid(),
        ],
        [
            acl.worker_oid(),
            routine.worker_oid(),
            rls.worker_oid(),
            trigger.worker_oid(),
            rights.worker_oid(),
        ],
        [
            acl.public_schema_oid(),
            routine.public_schema_oid(),
            rls.public_schema_oid(),
            trigger.public_schema_oid(),
        ],
        [
            routine.migration_owner_oid(),
            rls.migration_owner_oid(),
            trigger.migration_owner_oid(),
        ],
    ) {
        return Err(StartupFailure::Rejected);
    }
    runtime.revalidate().map_err(|_| ())?;
    let timeout_ms = limit_ms.min(30_000);
    let pool = official_pool(options, db, worker, timeout_ms);
    let queue = JobQueue::new(pool.clone(), None, QueueConfig::default());
    let recovered = owned_step(
        async {
            super::recover_simulation_claims(&queue)
                .await
                .map(|_| ())
                .map_err(OperationFailure::Recovery)
        },
        deadline,
        &mut shutdown,
        &canceled,
    )
    .await;
    if let Err(error) = recovered {
        pool.close().await;
        return Err(error.into());
    }
    if generation_check(&mut process, started, limit_ms).is_err()
        || process.generation() != generation
    {
        pool.close().await;
        return Err(StartupFailure::Rejected);
    }
    Ok(StartupEvidence {
        config,
        runtime,
        process,
        pool,
        queue,
        deadline,
        started_boot_ms: started,
        limit_ms,
        _approval: approved,
        _migration: migration,
        _acl: acl,
        _routine: routine,
        _rls: rls,
        _trigger: trigger,
        _rights: rights,
    })
}

fn official_pool(
    options: PgConnectOptions,
    database_oid: u32,
    worker_oid: u32,
    timeout_ms: u64,
) -> PgPool {
    PgPoolOptions::new().max_connections(5)
        .after_connect(move |connection, _| Box::pin(async move {
            sqlx::query("SELECT pg_catalog.set_config('search_path', 'pg_catalog, public', false), pg_catalog.set_config('statement_timeout', $1, false), pg_catalog.set_config('lock_timeout', $1, false)")
                .bind(timeout_ms.to_string()).fetch_one(&mut *connection).await?;
            if !session_valid(connection, database_oid, worker_oid, timeout_ms).await? { return Err(sqlx::Error::Protocol("simulation session policy rejected".into())); }
            Ok(())
        }))
        .before_acquire(move |connection, _| Box::pin(async move { session_valid(connection, database_oid, worker_oid, timeout_ms).await }))
        .connect_lazy_with(options)
}

async fn session_valid(
    connection: &mut PgConnection,
    database_oid: u32,
    worker_oid: u32,
    timeout_ms: u64,
) -> Result<bool, sqlx::Error> {
    let row: Option<(i64, i64, bool, bool, i64, i64, bool)> = sqlx::query_as("SELECT r.oid::pg_catalog.int8, d.oid::pg_catalog.int8, (current_user = 'worker' AND session_user = 'worker' AND pg_catalog.current_setting('role') = 'none' AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls), (pg_catalog.current_setting('search_path') = 'pg_catalog, public' AND pg_catalog.current_setting('row_security') = 'on' AND pg_catalog.current_setting('session_replication_role') = 'origin'), st.setting::pg_catalog.int8, lt.setting::pg_catalog.int8, (st.unit = 'ms' AND lt.unit = 'ms') FROM pg_catalog.pg_roles r JOIN pg_catalog.pg_database d ON d.datname = pg_catalog.current_database() JOIN pg_catalog.pg_settings st ON st.name = 'statement_timeout' JOIN pg_catalog.pg_settings lt ON lt.name = 'lock_timeout' WHERE r.rolname = 'worker'")
        .fetch_optional(connection).await?;
    Ok(session_matches(row, database_oid, worker_oid, timeout_ms))
}

fn session_matches(
    row: Option<(i64, i64, bool, bool, i64, i64, bool)>,
    db: u32,
    worker: u32,
    limit: u64,
) -> bool {
    matches!(row, Some((w, d, true, true, statement, lock, true))
        if w == i64::from(worker) && d == i64::from(db)
            && statement > 0 && lock > 0 && statement as u64 <= limit && lock as u64 <= limit)
}

#[cfg(test)]
mod tests;
