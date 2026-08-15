//! Todo 3 migration contract gate: schemas, roles, and immutable state
//! boundaries, proven against a disposable PostgreSQL 18 cluster.
//!
//! PLAN-COMMAND DEFECT (documented deviation): the plan's QA line for Todo 3 is
//! `cargo test -p api-server --test migration_contract`, but `apps/api-server`
//! is the Node/TypeScript application, NOT a Rust crate. This crate
//! (`migration-contract`, root workspace member) is the documented replacement:
//! it embeds `migrations/` via `sqlx::migrate!("../../../migrations")` (path is
//! relative to CARGO_MANIFEST_DIR = `tests/integration/migration-contract`) and
//! drives run/re-run/revert/denial assertions against a disposable database.
//!
//! Every test requires `DATABASE_URL` to point at a SUPERVISOR connection to a
//! DISPOSABLE PostgreSQL 18 cluster, e.g.
//! `postgres://postgres:lagrange@127.0.0.1:5432/postgres`. The tests run by
//! DEFAULT (un-gated since Todo 3's live gate passed): with `DATABASE_URL` set
//! they create/drop their own scratch databases and run the full contract; on
//! hosts with no database at all, `require_db_url()` below skips them cleanly
//! (reported as `ok`, zero assertions) instead of failing with a connection
//! error, so `cargo test --workspace` stays green everywhere.
//!
//! Covered contract (acceptance for plan Todo 3):
//!   - `sqlx migrate run` applies all migrations; a second run is a no-op.
//!   - `revert` (undo all) then `run` succeeds in a disposable DB.
//!   - every tenant table carries an ownership column (`owner_user_id`,
//!     `account_id`, `user_id`, or `created_by_user_id`).
//!   - every table is owned by `migration_owner`; `app`/`worker`/`audit_writer`
//!     have no table ownership and no BYPASSRLS.
//!   - public `jobs.status` has EXACTLY five values
//!     (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELED); a sixth (ORPHANED) is
//!     rejected by CHECK.
//!   - `job_attempts.outcome` includes `ORPHANED` (attempt-level only;
//!     CANCELED is rejected there).
//!   - app role denial: ALTER TABLE, TRUNCATE audit_logs, UPDATE/DELETE/INSERT
//!     audit_logs, INSERT into system-owned shared tables (cross-owner insert),
//!     sixth job status, CREATE TABLE.
//!   - audit_logs append-only: audit_writer may INSERT but never
//!     UPDATE/DELETE/TRUNCATE.
//!   - sha256-hash columns enforce `^[0-9a-f]{64}$`; web_sessions.session_hash
//!     is unique; data_entitlements lifecycle is CHECK-enforced.
//!   - large curves/orders/fills live in Parquet with DB manifests
//!     (result_artifacts: parquet_path + row_count + sha256 + summary_json).

use api_server_auth::postgres::{
    PostgresInviteStore, PostgresSessionStore, PostgresUserStore, with_actor_user_id,
};
use auth::audit::NoopAudit;
use auth::clock::FakeClock;
use auth::entitlement::{Role, UserId};
use auth::invites::{InviteRecord, InviteStore, RedeemedIdentity, UserRecord, UserStore};
use auth::oidc::{
    InMemoryPendingAuthStore, OidcClient, OidcProviderConfig, PendingAuth, PendingAuthStore,
};
use auth::service::AuthService;
use auth::sessions::SessionService;
use auth::simulator::{SIM_AUDIENCE, Simulator};
use sqlx::migrate::{MigrationType, Migrator};
use sqlx::postgres::PgPool;
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Uuid;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Migrations embedded at compile time from the workspace `migrations/` dir.
static MIGRATOR: Migrator = sqlx::migrate!("../../../migrations");

const SOURCE_INDEX_UP_SQL: &str =
    include_str!("../../../../migrations/0024_research_publication_source_index.up.sql");
const SOURCE_INDEX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0024_research_publication_source_index.down.sql");
const CALENDAR_VERSION_INDEX_UP_SQL: &str =
    include_str!("../../../../migrations/0025_research_calendar_version_lookup.up.sql");
const CALENDAR_VERSION_INDEX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0025_research_calendar_version_lookup.down.sql");
const RECOMMENDATION_PIPELINE_UP_SQL: &str =
    include_str!("../../../../migrations/0026_recommendation_pipeline.up.sql");
const RECOMMENDATION_PIPELINE_DOWN_SQL: &str =
    include_str!("../../../../migrations/0026_recommendation_pipeline.down.sql");
const RECOMMENDATION_ITEM_CONSTRAINT_UP_SQL: &str =
    include_str!("../../../../migrations/0030_recommendation_item_unique_constraint.up.sql");
const RECOMMENDATION_ITEM_CONSTRAINT_DOWN_SQL: &str =
    include_str!("../../../../migrations/0030_recommendation_item_unique_constraint.down.sql");
const RECOMMENDATION_ROLLBACK_GUARD_UP_SQL: &str =
    include_str!("../../../../migrations/0033_recommendation_rollback_guard.up.sql");
const RECOMMENDATION_ROLLBACK_GUARD_DOWN_SQL: &str =
    include_str!("../../../../migrations/0033_recommendation_rollback_guard.down.sql");
const RECOMMENDATION_PUBLICATION_LOCK_UP_SQL: &str =
    include_str!("../../../../migrations/0034_recommendation_publication_locks.up.sql");
const RECOMMENDATION_PUBLICATION_LOCK_DOWN_SQL: &str =
    include_str!("../../../../migrations/0034_recommendation_publication_locks.down.sql");
const RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL: &str =
    include_str!("../../../../migrations/0035_recommendation_entitlement_lock.up.sql");
const RECOMMENDATION_ENTITLEMENT_LOCK_DOWN_SQL: &str =
    include_str!("../../../../migrations/0035_recommendation_entitlement_lock.down.sql");
const RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL: &str =
    include_str!("../../../../migrations/0036_recommendation_submission_dataset_lock.up.sql");
const RECOMMENDATION_SUBMISSION_DATASET_LOCK_DOWN_SQL: &str =
    include_str!("../../../../migrations/0036_recommendation_submission_dataset_lock.down.sql");
const PAPER_RECOMMENDATION_EXECUTION_UP_SQL: &str =
    include_str!("../../../../migrations/0037_paper_recommendation_execution.up.sql");
const PAPER_RECOMMENDATION_EXECUTION_DOWN_SQL: &str =
    include_str!("../../../../migrations/0037_paper_recommendation_execution.down.sql");
const PAPER_REBALANCE_PREVIEW_UP_SQL: &str =
    include_str!("../../../../migrations/0038_paper_rebalance_previews.up.sql");
const PAPER_REBALANCE_PREVIEW_DOWN_SQL: &str =
    include_str!("../../../../migrations/0038_paper_rebalance_previews.down.sql");
const RESEARCH_PUBLICATION_DOWN_SQL: &str =
    include_str!("../../../../migrations/0022_research_publication.down.sql");
const RESEARCH_SCHEMA_GATE_SQL: &str =
    include_str!("../../../../deploy/compose/research-schema-check.sql");
const IDENTITY_PROVISIONING_UP_SQL: &str =
    include_str!("../../../../migrations/0040_identity_provisioning.up.sql");
const IDENTITY_PROVISIONING_DOWN_SQL: &str =
    include_str!("../../../../migrations/0040_identity_provisioning.down.sql");
const AUTH_AUDIT_OUTBOX_UP_SQL: &str =
    include_str!("../../../../migrations/0039_auth_audit_outbox.up.sql");
const AUTH_AUDIT_OUTBOX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0039_auth_audit_outbox.down.sql");
const PAPER_SETTLEMENT_OUTBOX_UP_SQL: &str =
    include_str!("../../../../migrations/0041_paper_settlement_outbox.up.sql");
const PAPER_SETTLEMENT_OUTBOX_DOWN_SQL: &str =
    include_str!("../../../../migrations/0041_paper_settlement_outbox.down.sql");
const PAPER_PARITY_REPO_RS: &str =
    include_str!("../../../../crates/api-server/src/repos/parity.rs");
const PAPER_PENDING_TARGET_REPO_RS: &str =
    include_str!("../../../../crates/api-server/src/repos/pending_targets.rs");

#[test]
fn paper_settlement_outbox_contract_is_deferred_and_fail_closed() {
    let (version, sql) = ("0038", PAPER_REBALANCE_PREVIEW_UP_SQL);
    let preflight = sql
        .split("CREATE FUNCTION public.preflight_paper_target")
        .nth(1)
        .or_else(|| {
            sql.split("CREATE OR REPLACE FUNCTION public.preflight_paper_target")
                .nth(1)
        })
        .expect("preflight implementation must exist")
        .split("ALTER FUNCTION public.preflight_paper_target")
        .next()
        .expect("preflight implementation must be bounded");
    assert!(
        !preflight.contains("UPDATE public.pending_targets")
            && !preflight.contains("SET status = 'SKIPPED'"),
        "{version} preflight must not commit a terminal target"
    );
    for token in [
        "paper_settlement_outbox",
        "paper_settlement_outbox_archive",
        "pending_targets_require_settlement_outbox",
        "DEFERRABLE INITIALLY DEFERRED",
        "terminal Paper target has no durable settlement outbox obligation",
        "Backfill every terminal row",
        "INSERT INTO public.paper_settlement_outbox",
        "preflight_paper_target",
        "Intentionally no UPDATE",
        "enqueue_paper_settlement_outbox",
        "v_target.owner_user_id",
        "SET search_path = pg_catalog, public",
        "REVOKE ALL ON TABLE public.paper_settlement_outbox",
        "GRANT EXECUTE ON FUNCTION public.enqueue_paper_settlement_outbox",
        "recommendation_runs_exact_lineage_uq",
        "pending_targets_recommendation_exact_lineage_fk",
        "paper_settlement_migration_validate_pending_targets",
        "DROP POLICY paper_settlement_migration_validate_pending_targets",
        "paper_settlement_migration_cleanup_notification_deliveries",
        "DROP POLICY paper_settlement_migration_cleanup_notification_deliveries",
        "notifications_id_owner_uq",
        "notification_deliveries_notification_owner_fk",
        "paper_settlement_outbox_stats",
        "oldest_pending_age_secs",
        "exhausted_count",
        "prune_paper_settlement_outbox",
        "LEAST(900",
        "Paper settlement archive payload mismatch",
    ] {
        assert!(
            PAPER_SETTLEMENT_OUTBOX_UP_SQL.contains(token),
            "0041 up is missing {token}"
        );
    }
    assert!(
        !PAPER_SETTLEMENT_OUTBOX_UP_SQL
            .contains("UPDATE public.pending_targets\n           SET status = 'SKIPPED'"),
        "preflight must not commit a terminal target"
    );
    for token in [
        "Paper settlement rollback blocked while pending outbox obligations exist",
        "terminal target without durable obligation",
        "DROP TABLE public.paper_settlement_outbox_archive",
        "DROP FUNCTION IF EXISTS public.enqueue_paper_settlement_outbox",
        "DROP CONSTRAINT IF EXISTS notification_deliveries_notification_channel_uq",
        "DROP CONSTRAINT IF EXISTS notification_deliveries_notification_owner_fk",
    ] {
        assert!(
            PAPER_SETTLEMENT_OUTBOX_DOWN_SQL.contains(token),
            "0041 down is missing {token}"
        );
    }
}

#[test]
fn paper_settlement_uses_exact_locked_lineage_and_idempotent_recovery() {
    let settlement = PAPER_PARITY_REPO_RS
        .split("pub(crate) async fn report_for_target_tx")
        .nth(1)
        .expect("settlement parity seam must exist");
    let settlement = settlement
        .split("fn missing_side")
        .next()
        .expect("settlement parity seam must have a bounded body");
    for token in [
        "target.recommendation_run_id",
        "run.owner_user_id = $2",
        "run.as_of = $4",
        "run.dataset_version_id = $5",
        "run.dataset_manifest_sha256 = $6",
        "FOR SHARE OF run, config, dataset",
        "recommendation_items",
        "owner_user_id = $2",
    ] {
        assert!(
            settlement.contains(token),
            "exact settlement lineage lost {token}"
        );
    }
    assert!(
        !settlement.contains("ORDER BY r.created_at DESC") && !settlement.contains("LIMIT 1"),
        "settlement may not fall back to the latest same-day recommendation"
    );

    for token in [
        "enqueue_paper_settlement_outbox",
        "mark_paper_settlement_outbox_delivered",
        "fail_paper_settlement_outbox",
        "paper_settlement_outbox_stats",
        "prune_paper_settlement_outbox",
        "delivered_at IS NULL AND exhausted_at IS NULL",
    ] {
        assert!(
            PAPER_PENDING_TARGET_REPO_RS.contains(token),
            "repository recovery seam is missing {token}"
        );
    }
}

/// DB-gated seam test for the irreversible parts of 0041.  It intentionally
/// skips when no disposable PostgreSQL supervisor is configured; CI's
/// migration lane supplies DATABASE_URL and exercises the real RLS/grants.
#[tokio::test]
async fn paper_settlement_outbox_db_contract() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = paper_settlement_outbox_db_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("Paper settlement outbox DB contract FAILED: {error}");
    }
}

async fn paper_settlement_outbox_db_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run_to(40, owner).await?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'paper-outbox-contract', 'paper-outbox@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('paper_outbox_contract', 'Paper Outbox Contract', 'Paper')",
    )
    .execute(owner)
    .await?;
    let owner_actor = actor_pool(super_url, db, "migration_owner", &user_id.to_string()).await?;
    let worker = effective_role_pool(super_url, db, "worker", None, 2).await?;
    let app = actor_pool(super_url, db, "app", &user_id.to_string()).await?;
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'paper_outbox_contract', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, status) \
         VALUES ($1, 'PAPER', 'paper-outbox-contract', 'ACTIVE') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let pending_id: Uuid = sqlx::query_scalar(
        "INSERT INTO pending_targets \
         (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, targets_json) \
         VALUES ($1, $2, $3, DATE '2026-08-10', DATE '2026-08-11', '[]'::jsonb) \
         RETURNING id",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .fetch_one(&owner_actor)
    .await?;
    let terminal_id: Uuid = sqlx::query_scalar(
        "INSERT INTO pending_targets \
         (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, \
          targets_json, status, executed_at, non_execution_reason) \
         VALUES ($1, $2, $3, DATE '2026-08-12', DATE '2026-08-13', '[]'::jsonb, \
                 'SKIPPED', now(), '{\"code\":\"PAPER_LEGACY\",\"message\":\"legacy\"}'::jsonb) \
         RETURNING id",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .fetch_one(&owner_actor)
    .await?;

    // A direct worker preflight denial is read-only: it cannot commit a
    // terminal target or manufacture an orphan.
    let preflight: (bool, serde_json::Value) =
        sqlx::query_as("SELECT authorized, reason FROM preflight_paper_target($1, $2)")
            .bind(pending_id)
            .bind(user_id)
            .fetch_one(&worker)
            .await?;
    assert!(!preflight.0);
    let pending_status: String =
        sqlx::query_scalar("SELECT status FROM pending_targets WHERE id = $1")
            .bind(pending_id)
            .fetch_one(&owner_actor)
            .await?;
    assert_eq!(pending_status, "PENDING");

    MIGRATOR.run_to(41, owner).await?;
    let backfilled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM paper_settlement_outbox WHERE pending_target_id = $1",
    )
    .bind(terminal_id)
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(backfilled, 1, "0041 must backfill terminal legacy rows");

    // The worker has a target UPDATE grant for its execution role, but the
    // deferred trigger still rejects a direct terminal transition without an
    // outbox row.  This is the DB-level guard behind the read-only preflight.
    let mut orphan_tx = worker.begin().await?;
    sqlx::query(
        "UPDATE pending_targets SET status = 'SKIPPED', executed_at = now(), \
                non_execution_reason = '{\"code\":\"PAPER_DIRECT\",\"message\":\"direct\"}'::jsonb \
         WHERE id = $1",
    )
    .bind(pending_id)
    .execute(&mut *orphan_tx)
    .await?;
    let orphan_commit = orphan_tx.commit().await.unwrap_err();
    assert_eq!(pg_code(&orphan_commit).as_deref(), Some("23514"));

    let direct_insert = sqlx::query(
        "INSERT INTO paper_settlement_outbox \
         (pending_target_id, owner_user_id, severity, kind, title) \
         VALUES ($1, $2, 'INFO', 'job', 'squat')",
    )
    .bind(terminal_id)
    .bind(user_id)
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&direct_insert).as_deref(), Some("42501"));

    // The definer derives owner from the locked target.  A foreign actor
    // cannot use the same target id to squat the tenant key.
    let foreign_user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'paper-outbox-foreign', 'paper-foreign@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    let foreign_app = actor_pool(super_url, db, "app", &foreign_user.to_string()).await?;
    let foreign_enqueue =
        sqlx::query("SELECT enqueue_paper_settlement_outbox($1, 'INFO', 'job', 'squat', '', NULL)")
            .bind(terminal_id)
            .execute(&foreign_app)
            .await
            .unwrap_err();
    assert_eq!(pg_code(&foreign_enqueue).as_deref(), Some("42501"));
    let foreign_notification: Uuid = sqlx::query_scalar(
        "INSERT INTO notifications (owner_user_id, kind, title) \
         VALUES ($1, 'alert', 'foreign') RETURNING id",
    )
    .bind(foreign_user)
    .fetch_one(&foreign_app)
    .await?;
    let foreign_delivery = sqlx::query(
        "INSERT INTO notification_deliveries \
         (notification_id, owner_user_id, channel, status) \
         VALUES ($1, $2, 'web', 'SUCCESS')",
    )
    .bind(foreign_notification)
    .bind(user_id)
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&foreign_delivery).as_deref(), Some("23503"));

    // A target transition and enqueue may be ordered either way inside one
    // transaction, but neither can commit alone.  This also gives the retry
    // and pruning assertions an owned terminal target without bypassing the
    // deferred invariant.
    let recovery_target_id: Uuid;
    let recovery_outbox_id: Uuid;
    {
        let mut tx = owner_actor.begin().await?;
        recovery_target_id = sqlx::query_scalar(
            "INSERT INTO pending_targets \
             (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, \
              targets_json, status, executed_at, non_execution_reason) \
             VALUES ($1, $2, $3, DATE '2026-08-14', DATE '2026-08-15', '[]'::jsonb, \
                     'SKIPPED', now(), '{\"code\":\"PAPER_RECOVERY\",\"message\":\"recovery\"}'::jsonb) \
             RETURNING id",
        )
        .bind(account_id)
        .bind(user_id)
        .bind(config_id)
        .fetch_one(&mut *tx)
        .await?;
        recovery_outbox_id = sqlx::query_scalar(
            "SELECT enqueue_paper_settlement_outbox(\
                 $1, 'WARNING', 'alert', 'recovery', 'retryable', NULL)",
        )
        .bind(recovery_target_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    sqlx::query("UPDATE paper_settlement_outbox SET max_attempts = 2 WHERE id = $1")
        .bind(recovery_outbox_id)
        .execute(&owner_actor)
        .await?;
    let first_failure: (i32, bool) = sqlx::query_as(
        "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'timeout')",
    )
    .bind(recovery_outbox_id)
    .bind(user_id)
    .fetch_one(&app)
    .await?;
    assert_eq!(first_failure, (1, false));
    let second_failure: (i32, bool) = sqlx::query_as(
        "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'timeout')",
    )
    .bind(recovery_outbox_id)
    .bind(user_id)
    .fetch_one(&app)
    .await?;
    assert_eq!(
        second_failure,
        (2, true),
        "retry limit must exhaust exactly once"
    );
    let exhausted_stats: (i64, i64, bool) = sqlx::query_as(
        "SELECT pending_count, exhausted_count, ready \
         FROM paper_settlement_outbox_stats(900)",
    )
    .fetch_one(&worker)
    .await?;
    assert!(exhausted_stats.0 >= 1);
    assert!(exhausted_stats.1 >= 1);
    assert!(!exhausted_stats.2, "exhausted delivery blocks readiness");

    // Delivery marking is idempotent: a timeout/restart replay can call the
    // DB seam twice, but only the first call changes the obligation.
    let first_mark: bool =
        sqlx::query_scalar("SELECT mark_paper_settlement_outbox_delivered($1, $2)")
            .bind(recovery_outbox_id)
            .bind(user_id)
            .fetch_one(&app)
            .await?;
    let second_mark: bool =
        sqlx::query_scalar("SELECT mark_paper_settlement_outbox_delivered($1, $2)")
            .bind(recovery_outbox_id)
            .bind(user_id)
            .fetch_one(&app)
            .await?;
    assert!(first_mark);
    assert!(!second_mark);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM paper_settlement_outbox WHERE pending_target_id = $1",
        )
        .bind(recovery_target_id)
        .fetch_one(&owner_actor)
        .await?,
        1
    );

    // The retention boundary archives before deleting the active delivered
    // row.  The terminal target remains covered by the archive invariant.
    sqlx::query(
        "UPDATE paper_settlement_outbox \
         SET delivered_at = now() - interval '86401 seconds' \
         WHERE id = $1",
    )
    .bind(recovery_outbox_id)
    .execute(&owner_actor)
    .await?;
    let pruned: i64 = sqlx::query_scalar("SELECT prune_paper_settlement_outbox(86400, 256)")
        .fetch_one(&worker)
        .await?;
    assert_eq!(pruned, 1);
    let retained: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM paper_settlement_outbox WHERE pending_target_id = $1), \
           (SELECT count(*) FROM paper_settlement_outbox_archive WHERE pending_target_id = $1)",
    )
    .bind(recovery_target_id)
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(retained, (0, 1));
    let replay_id: Uuid = sqlx::query_scalar(
        "SELECT enqueue_paper_settlement_outbox(\
             $1, 'WARNING', 'alert', 'recovery', 'retryable', NULL)",
    )
    .bind(recovery_target_id)
    .fetch_one(&app)
    .await?;
    assert_eq!(replay_id, recovery_outbox_id);
    let active_after_replay: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM paper_settlement_outbox WHERE pending_target_id = $1",
    )
    .bind(recovery_target_id)
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(
        active_after_replay, 0,
        "archive replay must not recreate active work"
    );

    let mut guarded = owner.acquire().await?;
    let down_error = MIGRATOR.undo(&mut *guarded, 41).await.unwrap_err();
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *guarded)
        .await?;
    drop(guarded);
    assert_eq!(migrate_pg_code(&down_error).as_deref(), Some("55000"));
    sqlx::query(
        "UPDATE paper_settlement_outbox SET delivered_at = now() WHERE pending_target_id = $1",
    )
    .bind(terminal_id)
    .execute(&owner_actor)
    .await?;
    MIGRATOR.undo(owner, 41).await?;
    Ok(())
}

#[test]
fn auth_audit_outbox_is_transactional_and_reversible() {
    for token in [
        "auth_audit_outbox",
        "event_key",
        "UNIQUE",
        "delivered_at",
        "ENABLE ROW LEVEL SECURITY",
        "FORCE ROW LEVEL SECURITY",
        "enqueue_auth_audit",
        "SECURITY DEFINER",
        "SET search_path = pg_catalog, pg_temp",
        "ON CONFLICT (event_key) DO NOTHING",
        "auth audit event key payload mismatch",
        "GRANT EXECUTE ON FUNCTION",
        "TO app, audit_writer",
        "deliver_auth_audit_batch",
        "delivered_count integer",
        "failed_count integer",
        "available_at = pg_catalog.clock_timestamp()",
        "FOR UPDATE",
        "SKIP LOCKED",
        "auth_audit_outbox_stats",
        "oldest_pending_age_secs",
        "prune_auth_audit_outbox",
        "delivered_at IS NOT NULL",
        "auth_audit_log_insert_migration_owner",
        "set_config('statement_timeout', '5000', true)",
        "set_config('statement_timeout', '3000', true)",
        "set_config('lock_timeout', '1000', true)",
        "SET lock_timeout = '1s'",
        "SET statement_timeout = '5s'",
        "SET statement_timeout = '3s'",
    ] {
        assert!(
            AUTH_AUDIT_OUTBOX_UP_SQL.contains(token),
            "0039 outbox up is missing {token}"
        );
    }
    for token in [
        "DROP FUNCTION public.enqueue_auth_audit",
        "DROP POLICY IF EXISTS auth_audit_outbox_owner_update",
        "DROP TABLE public.auth_audit_outbox",
        "DROP POLICY IF EXISTS auth_audit_log_insert_migration_owner",
    ] {
        assert!(
            AUTH_AUDIT_OUTBOX_DOWN_SQL.contains(token),
            "0039 outbox down is missing {token}"
        );
    }
    assert!(
        !AUTH_AUDIT_OUTBOX_UP_SQL.contains("GRANT SELECT, UPDATE ON public.auth_audit_outbox"),
        "audit_writer must not receive direct outbox table DML"
    );
}

#[test]
fn auth_outbox_migration_precedes_identity_consumers_and_rolls_back_first() {
    let up39 = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 39 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0039 outbox migration is embedded");
    let up40 = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 40 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0040 identity migration is embedded");
    assert!(
        up39.sql
            .as_str()
            .contains("CREATE TABLE public.auth_audit_outbox")
    );
    assert!(
        up40.sql
            .as_str()
            .contains("CREATE FUNCTION public.create_invitation")
    );
    assert!(up39.version < up40.version);
    let down39 = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 39 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0039 outbox rollback is embedded");
    let down40 = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 40 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0040 identity rollback is embedded");
    assert!(
        down40
            .sql
            .as_str()
            .contains("DROP FUNCTION public.create_invitation")
    );
    assert!(
        down39
            .sql
            .as_str()
            .contains("DROP TABLE public.auth_audit_outbox")
    );
}

#[test]
fn identity_provisioning_migration_is_narrow_and_reversible() {
    for token in [
        "invitations",
        "role_id",
        "provisioned_by_user_id",
        "invitations_pending_email_uq",
        "create_invitation",
        "claim_invitation",
        "bind_redeemed_identity",
        "SECURITY DEFINER",
        "SET search_path = pg_catalog, pg_temp",
        "REVOKE ALL ON FUNCTION",
        "GRANT EXECUTE ON FUNCTION",
        "TO app",
        "pg_advisory_xact_lock",
        "expire_pending_invitations",
        "tenant_all_owner_invitations",
        "status = 'EXPIRED'",
        "expires_at <= pg_catalog.clock_timestamp()",
        "FOR UPDATE",
        "ON CONFLICT (issuer, subject) DO NOTHING",
        "duplicate pending invitation emails require manual resolution",
        "lower(pg_catalog.btrim(email))",
        "provisioned_by_user_id = NULL",
        "IS DISTINCT FROM",
        "an identity already exists for this email",
        "existing_user.email",
        "pg_catalog.hashtextextended(v_email, 39039)",
        "pg_catalog.lower(pg_catalog.btrim(v_invitation.email))",
    ] {
        assert!(
            IDENTITY_PROVISIONING_UP_SQL.contains(token),
            "0040 identity up is missing {token}"
        );
    }
    let expire_position = IDENTITY_PROVISIONING_UP_SQL
        .find("status = 'EXPIRED'")
        .expect("0040 expires stale pending invitations");
    let duplicate_position = IDENTITY_PROVISIONING_UP_SQL
        .find("\n    IF EXISTS (\n        SELECT 1\n        FROM public.invitations AS invitation")
        .expect("0040 duplicate check remains explicit");
    assert!(
        expire_position < duplicate_position,
        "expired pending invitations must be released before duplicate detection"
    );
    let create_start = IDENTITY_PROVISIONING_UP_SQL
        .find("CREATE FUNCTION public.create_invitation")
        .expect("create_invitation definition");
    let create_end = IDENTITY_PROVISIONING_UP_SQL[create_start..]
        .find("ALTER FUNCTION public.create_invitation")
        .map(|offset| create_start + offset)
        .expect("create_invitation metadata");
    let create_sql = &IDENTITY_PROVISIONING_UP_SQL[create_start..create_end];
    let create_lock = create_sql
        .find("pg_catalog.hashtextextended(v_email, 39039)")
        .expect("create takes normalized-email lock");
    let create_user_check = create_sql
        .find("FROM public.users AS existing_user")
        .expect("create has global user check");
    assert!(
        create_lock < create_user_check,
        "create must lock before its global users check"
    );
    let claim_start = IDENTITY_PROVISIONING_UP_SQL
        .find("CREATE FUNCTION public.claim_invitation")
        .expect("claim_invitation definition");
    let claim_end = IDENTITY_PROVISIONING_UP_SQL[claim_start..]
        .find("ALTER FUNCTION public.claim_invitation")
        .map(|offset| claim_start + offset)
        .expect("claim_invitation metadata");
    let claim_sql = &IDENTITY_PROVISIONING_UP_SQL[claim_start..claim_end];
    let claim_lock = claim_sql
        .find("pg_catalog.hashtextextended(")
        .expect("claim takes normalized-email lock");
    let claim_email_check = claim_sql
        .find("FROM public.users\n        WHERE pg_catalog.lower")
        .expect("claim has global normalized-email check");
    assert!(
        claim_lock < claim_email_check,
        "claim must lock before its global users/provisional check"
    );
    let bind_start = IDENTITY_PROVISIONING_UP_SQL
        .find("CREATE FUNCTION public.bind_redeemed_identity")
        .expect("bind_redeemed_identity definition");
    let bind_end = IDENTITY_PROVISIONING_UP_SQL[bind_start..]
        .find("ALTER FUNCTION public.bind_redeemed_identity")
        .map(|offset| bind_start + offset)
        .expect("bind_redeemed_identity metadata");
    let bind_sql = &IDENTITY_PROVISIONING_UP_SQL[bind_start..bind_end];
    let bind_lock = bind_sql
        .find("pg_catalog.hashtextextended(v_email, 39039)")
        .expect("bind takes normalized-email lock");
    let bind_user_check = bind_sql
        .find("FROM public.users")
        .expect("bind has global users check");
    assert!(
        bind_lock < bind_user_check,
        "bind must lock before its global users/provisional check"
    );
    assert!(
        !IDENTITY_PROVISIONING_UP_SQL.contains("SET search_path = pg_catalog, public"),
        "SECURITY DEFINER functions must not search the writable public schema"
    );
    assert!(
        !IDENTITY_PROVISIONING_UP_SQL.contains("GRANT INSERT ON TABLE public.users")
            && !IDENTITY_PROVISIONING_UP_SQL.contains("GRANT INSERT ON TABLE public.user_roles"),
        "identity provisioning must not add table write grants"
    );
    for token in [
        "DROP FUNCTION public.bind_redeemed_identity",
        "DROP FUNCTION public.claim_invitation",
        "DROP FUNCTION public.expire_pending_invitations",
        "DROP FUNCTION public.create_invitation",
        "DROP COLUMN role_id",
        "DROP COLUMN provisioned_by_user_id",
        "DROP INDEX public.invitations_pending_email_uq",
        "cannot roll back identity provisioning while provisional identities exist",
        "cannot roll back identity provisioning while Owner invitations remain",
        "WHERE role_id <> 'member'",
        "NO FORCE ROW LEVEL SECURITY",
        "FORCE ROW LEVEL SECURITY",
    ] {
        assert!(
            IDENTITY_PROVISIONING_DOWN_SQL.contains(token),
            "0040 down is missing {token}"
        );
    }
    for signature in [
        "public.create_invitation(uuid, text, text, text, bigint)",
        "public.claim_invitation(uuid, uuid, text, text)",
        "public.bind_redeemed_identity(text, text, text, text)",
    ] {
        assert!(
            IDENTITY_PROVISIONING_DOWN_SQL.contains(signature),
            "0040 down must revoke {signature} explicitly"
        );
    }
}

#[tokio::test]
async fn identity_provisioning_is_atomic_and_role_scoped() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = identity_provisioning_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("identity provisioning contract FAILED: {error}");
    }
}

async fn identity_provisioning_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run(owner).await?;

    let owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'provision-owner', 'provision-owner@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id, granted_by) VALUES ($1, 'owner', $1)")
        .bind(owner_id)
        .execute(owner)
        .await?;
    let member_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'provision-member', 'provision-member@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id, granted_by) VALUES ($1, 'member', $2)")
        .bind(member_id)
        .bind(owner_id)
        .execute(owner)
        .await?;
    let other_owner_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'provision-owner-2', 'provision-owner-2@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id, granted_by) VALUES ($1, 'owner', $1)")
        .bind(other_owner_id)
        .execute(owner)
        .await?;

    let app = role_pool(super_url, db, "app").await?;
    let admin = role_pool(super_url, db, "admin").await?;
    let audit = role_pool(super_url, db, "audit_writer").await?;
    let expires_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64 + 3_600;
    let owner_invitation_id: Uuid = sqlx::query_scalar(
        "SELECT public.create_invitation($1, 'owner-rollback@example.test', 'owner', $2, $3)",
    )
    .bind(owner_id)
    .bind("0f".repeat(32))
    .bind(expires_at)
    .fetch_one(&app)
    .await?;
    let invite_hash = "a1".repeat(32);
    let invitation_id: Uuid =
        sqlx::query_scalar("SELECT public.create_invitation($1, $2, $3, $4, $5)")
            .bind(owner_id)
            .bind("new-member@example.test")
            .bind("member")
            .bind(&invite_hash)
            .bind(expires_at)
            .fetch_one(&app)
            .await?;
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM auth_audit_outbox \
         WHERE event_key = $1 AND action = 'auth.invite_created'",
    )
    .bind(format!("invite:{invitation_id}:created"))
    .fetch_one(owner)
    .await?;
    assert_eq!(outbox_count, 1, "invite mutation must enqueue atomically");

    // The event key is idempotent only for an identical immutable payload;
    // replay returns the original id while a forged payload is rejected.
    let event_key = format!("contract:{}", Uuid::new_v4());
    let created_at = expires_at;
    let mut event_tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *event_tx)
        .await?;
    let first_id: Uuid = sqlx::query_scalar(
        "SELECT public.enqueue_auth_audit($1, 'auth.contract_probe', $2, 'test', $3, NULL, $4)",
    )
    .bind(&event_key)
    .bind(owner_id)
    .bind("same")
    .bind(created_at)
    .fetch_one(&mut *event_tx)
    .await?;
    let replay_id: Uuid = sqlx::query_scalar(
        "SELECT public.enqueue_auth_audit($1, 'auth.contract_probe', $2, 'test', $3, NULL, $4)",
    )
    .bind(&event_key)
    .bind(owner_id)
    .bind("same")
    .bind(created_at)
    .fetch_one(&mut *event_tx)
    .await?;
    event_tx.commit().await?;
    assert_eq!(replay_id, first_id);
    let mut mismatch_tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *mismatch_tx)
        .await?;
    let mismatch = sqlx::query(
        "SELECT public.enqueue_auth_audit($1, 'auth.tampered', $2, 'test', $3, NULL, $4)",
    )
    .bind(&event_key)
    .bind(owner_id)
    .bind("same")
    .bind(created_at)
    .fetch_one(&mut *mismatch_tx)
    .await
    .unwrap_err();
    mismatch_tx.rollback().await?;
    assert_eq!(pg_code(&mismatch).as_deref(), Some("23505"));

    let stats: (i64, i64) = sqlx::query_as(
        "SELECT pending_count, oldest_pending_age_secs \
         FROM public.auth_audit_outbox_stats()",
    )
    .fetch_one(&audit)
    .await?;
    assert!(stats.0 >= 2);
    let delivered: i32 = sqlx::query_scalar("SELECT public.deliver_auth_audit_batch(64)")
        .fetch_one(&audit)
        .await?;
    assert!(delivered >= 2);
    let copied: i64 = sqlx::query_scalar("SELECT count(*)::bigint FROM audit_logs WHERE id = $1")
        .bind(first_id)
        .fetch_one(owner)
        .await?;
    assert_eq!(copied, 1, "delivery must copy before marking delivered");
    let retention_target: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM auth_audit_outbox WHERE id = $1 AND delivered_at IS NOT NULL",
    )
    .bind(first_id)
    .fetch_one(owner)
    .await?;
    assert_eq!(retention_target, 1);
    sqlx::query(
        "UPDATE auth_audit_outbox SET delivered_at = now() - interval '2 days' WHERE id = $1",
    )
    .bind(first_id)
    .execute(owner)
    .await?;
    let pruned: i64 = sqlx::query_scalar("SELECT public.prune_auth_audit_outbox(86400, 256)")
        .fetch_one(&audit)
        .await?;
    assert!(pruned >= 1);
    let audit_survives: i64 =
        sqlx::query_scalar("SELECT count(*)::bigint FROM audit_logs WHERE id = $1")
            .bind(first_id)
            .fetch_one(owner)
            .await?;
    assert_eq!(
        audit_survives, 1,
        "retention must never remove immutable audit copy"
    );

    let duplicate = sqlx::query("SELECT public.create_invitation($1, $2, $3, $4, $5)")
        .bind(owner_id)
        .bind("new-member@example.test")
        .bind("member")
        .bind("b2".repeat(32))
        .bind(expires_at)
        .fetch_one(&app)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&duplicate).as_deref(), Some("23505"));

    // A stale PENDING row must be released while the same email advisory
    // lock is held, so re-invitation is safe and does not require manual DB
    // cleanup.
    let stale_id: Uuid = sqlx::query_scalar(
        "INSERT INTO invitations (user_id, email, invite_hash, status, expires_at, role_id) \
         VALUES ($1, 'stale@example.test', $2, 'PENDING', now() - interval '1 second', 'member') \
         RETURNING id",
    )
    .bind(owner_id)
    .bind("d4".repeat(32))
    .fetch_one(owner)
    .await?;
    let replacement_id: Uuid = sqlx::query_scalar(
        "SELECT public.create_invitation($1, 'stale@example.test', 'member', $2, $3)",
    )
    .bind(other_owner_id)
    .bind("e5".repeat(32))
    .bind(expires_at)
    .fetch_one(&app)
    .await?;
    assert_ne!(replacement_id, stale_id);
    let stale_status: String = sqlx::query_scalar("SELECT status FROM invitations WHERE id = $1")
        .bind(stale_id)
        .fetch_one(owner)
        .await?;
    assert_eq!(stale_status, "EXPIRED");

    let forged_owner = sqlx::query("SELECT public.create_invitation($1, $2, $3, $4, $5)")
        .bind(member_id)
        .bind("forged@example.test")
        .bind("member")
        .bind("c3".repeat(32))
        .bind(expires_at)
        .fetch_one(&app)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&forged_owner).as_deref(), Some("42501"));

    // Existing identities are globally ineligible for invitations, including
    // a provisional identity redeemed under a different Owner tenant.
    let existing_user = sqlx::query(
        "SELECT public.create_invitation($1, 'provision-member@example.test', 'member', $2, $3)",
    )
    .bind(owner_id)
    .bind("f1".repeat(32))
    .bind(expires_at)
    .fetch_one(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&existing_user).as_deref(), Some("23505"));
    let provisional_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email, provisioned_by_user_id) \
         VALUES ('https://issuer.test', 'provisional-existing', 'provisional-existing@example.test', $1) \
         RETURNING id",
    )
    .bind(owner_id)
    .fetch_one(owner)
    .await?;
    let provisional_invite = sqlx::query(
        "SELECT public.create_invitation($1, 'provisional-existing@example.test', 'member', $2, $3)",
    )
    .bind(other_owner_id)
    .bind("f2".repeat(32))
    .bind(expires_at)
    .fetch_one(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&provisional_invite).as_deref(), Some("23505"));
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(provisional_id)
        .execute(owner)
        .await?;

    let direct_user_insert = sqlx::query(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'direct-app', 'direct-app@example.test')",
    )
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&direct_user_insert).as_deref(), Some("42501"));

    // The create-vs-claim race uses separate app connections and one
    // normalized address.  Whichever operation wins the advisory fence, the
    // loser must observe the committed identity/ invitation state and no
    // duplicate pending invitation or provisional user may result.
    let race_email = format!("race-{}@example.test", Uuid::new_v4().simple());
    let race_invitation_id: Uuid =
        sqlx::query_scalar("SELECT public.create_invitation($1, $2, 'member', $3, $4)")
            .bind(owner_id)
            .bind(&race_email)
            .bind("ab".repeat(32))
            .bind(expires_at)
            .fetch_one(&app)
            .await?;
    let race_create_pool = role_pool(super_url, db, "app").await?;
    let race_claim_pool = role_pool(super_url, db, "app").await?;
    let race_email_for_create = race_email.clone();
    let create_race = async move {
        sqlx::query("SELECT public.create_invitation($1, $2, 'member', $3, $4)")
            .bind(owner_id)
            .bind(race_email_for_create)
            .bind("cd".repeat(32))
            .bind(expires_at)
            .fetch_one(&race_create_pool)
            .await
    };
    let claim_race = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4)")
            .bind(owner_id)
            .bind(race_invitation_id)
            .bind("https://issuer.test")
            .bind("auth0|create-vs-claim")
            .fetch_one(&race_claim_pool)
            .await
    };
    let (create_race, claim_race) = tokio::join!(create_race, claim_race);
    let create_race = create_race.expect_err("racing creation must lose the email fence");
    assert_eq!(pg_code(&create_race).as_deref(), Some("23505"));
    assert!(
        claim_race?,
        "the pending invitation must be claimed exactly once"
    );
    let race_counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM invitations WHERE lower(btrim(email)) = lower(btrim($1))), \
            (SELECT count(*) FROM users WHERE lower(btrim(email)) = lower(btrim($1))), \
            (SELECT count(*) FROM users WHERE issuer = 'https://issuer.test' \
                AND subject = 'auth0|create-vs-claim')",
    )
    .bind(&race_email)
    .fetch_one(&admin)
    .await?;
    assert_eq!(race_counts, (1, 1, 1));
    let race_user_id: Uuid =
        sqlx::query_scalar("SELECT redeemed_by_user_id FROM invitations WHERE id = $1")
            .bind(race_invitation_id)
            .fetch_one(&admin)
            .await?;
    sqlx::query("SELECT public.bind_redeemed_identity($1, $2, $3, 'member')")
        .bind("https://issuer.test")
        .bind("auth0|create-vs-claim")
        .bind(&race_email)
        .fetch_one(&app)
        .await?;
    let race_provenance: Option<Uuid> =
        sqlx::query_scalar("SELECT provisioned_by_user_id FROM users WHERE id = $1")
            .bind(race_user_id)
            .fetch_one(&admin)
            .await?;
    assert!(race_provenance.is_none());

    // Exercise the production adapters, not only the SQL procedures. The
    // actor is supplied through the same request-scoped context used by the
    // HTTP Owner invite handler; the serving connection still has no table
    // write grant.
    let adapter_invites = PostgresInviteStore::new(app.clone(), admin.clone());
    let adapter_invite = InviteRecord {
        id: format!("inv-{}", "d4".repeat(32)),
        email: "adapter-member@example.test".to_string(),
        role: Role::Member,
        created_at_secs: expires_at - 3_600,
        expires_at_secs: expires_at,
        redeemed_by: None,
        redeemed_at_secs: None,
    };
    with_actor_user_id(
        &UserId::new(owner_id.to_string()),
        adapter_invites.insert(adapter_invite),
    )
    .await?;
    let adapter_pending = adapter_invites
        .find_by_email("adapter-member@example.test")
        .await?
        .expect("adapter invitation");
    let adapter_first_id = adapter_pending.id.clone();
    let adapter_second_id = adapter_pending.id;
    let adapter_first_store = adapter_invites.clone();
    let adapter_second_store = adapter_invites.clone();
    let adapter_first = async move {
        adapter_first_store
            .claim(
                &adapter_first_id,
                "https://issuer.test",
                "auth0|adapter-member",
                expires_at - 3_500,
            )
            .await
    };
    let adapter_second = async move {
        adapter_second_store
            .claim(
                &adapter_second_id,
                "https://issuer.test",
                "auth0|adapter-member",
                expires_at - 3_500,
            )
            .await
    };
    let (adapter_first, adapter_second) = tokio::join!(adapter_first, adapter_second);
    let adapter_claimed = [adapter_first?, adapter_second?];
    assert!(
        adapter_claimed.iter().all(|claimed| *claimed),
        "same-identity retries may acknowledge one atomic provisional claim"
    );
    let adapter_users = PostgresUserStore::new(app.clone(), admin.clone());
    let canonical_adapter_id = adapter_users
        .insert_user(UserRecord {
            binding_issuer: "https://issuer.test".to_string(),
            binding_subject: "auth0|adapter-member".to_string(),
            user_id: UserId::new("usr_adapter_placeholder"),
            role: Role::Member,
            email: "adapter-member@example.test".to_string(),
            created_at_secs: expires_at - 3_500,
        })
        .await?;
    assert_eq!(
        canonical_adapter_id,
        adapter_users
            .find_by_binding("https://issuer.test", "auth0|adapter-member")
            .await?
            .expect("adapter identity")
            .user_id
    );
    assert!(
        Uuid::parse_str(&canonical_adapter_id.0).is_ok(),
        "production adapter must return the database UUID, not the provisional core ID"
    );
    let adapter_identity = adapter_users
        .find_by_binding("https://issuer.test", "auth0|adapter-member")
        .await?
        .expect("adapter identity");
    assert_eq!(adapter_identity.role, Role::Member);

    // The canonical ID returned by UserStore is the one that reaches the
    // durable session store. This is the production failure boundary: a
    // provisional `usr_<hex>` ID must never be handed to a UUID column.
    let session_service = SessionService::new(
        std::sync::Arc::new(PostgresSessionStore::new(app.clone(), admin.clone())),
        std::sync::Arc::new(FakeClock(expires_at - 3_500)),
        std::sync::Arc::new(NoopAudit),
    );
    let issued = session_service
        .issue(
            &RedeemedIdentity {
                user_id: canonical_adapter_id.clone(),
                role: Role::Member,
                email: "adapter-member@example.test".to_string(),
                binding: "https://issuer.test|auth0|adapter-member".to_string(),
            },
            expires_at - 3_500,
            vec!["pwd".to_string()],
        )
        .await?;
    let persisted = session_service.validate(&issued.cookie_value).await?;
    assert_eq!(persisted.user_id, canonical_adapter_id);

    // A direct bind replay must fail closed once provenance has been cleared;
    // normal login retries take the established find_by_binding path below.
    let retry_error = adapter_users
        .insert_user(UserRecord {
            binding_issuer: "https://issuer.test".to_string(),
            binding_subject: "auth0|adapter-member".to_string(),
            user_id: UserId::new("usr_retry_placeholder"),
            role: Role::Member,
            email: "adapter-member@example.test".to_string(),
            created_at_secs: expires_at - 3_400,
        })
        .await
        .expect_err("direct bind replay must not mutate an established identity");
    assert!(
        matches!(retry_error, auth::invites::InviteError::Store(_)),
        "adapter must not expose database details for a rejected bind replay"
    );

    // Drive the complete first-login callback through the production stores:
    // the simulator supplies a signed OIDC response, while Postgres owns the
    // invitation claim, canonical UUID binding, and durable session. A second
    // callback with the consumed state is rejected, and a fresh login for the
    // established immutable identity keeps the same UUID.
    let callback_invites = PostgresInviteStore::new(app.clone(), admin.clone());
    with_actor_user_id(
        &UserId::new(owner_id.to_string()),
        callback_invites.insert(InviteRecord {
            id: format!("inv-{}", "e5".repeat(32)),
            email: "callback-member@example.test".to_string(),
            role: Role::Member,
            created_at_secs: expires_at - 3_200,
            expires_at_secs: expires_at,
            redeemed_by: None,
            redeemed_at_secs: None,
        }),
    )
    .await?;
    let callback_users = PostgresUserStore::new(app.clone(), admin.clone());
    let callback_audit = std::sync::Arc::new(NoopAudit);
    let callback_simulator = std::sync::Arc::new(Simulator::new(
        "https://issuer.test",
        "migration-test-client",
        "https://app.lagrange.local/auth/callback",
    ));
    let callback_auth = AuthService::new(
        OidcClient {
            config: OidcProviderConfig {
                issuer: "https://issuer.test".to_string(),
                client_id: "migration-test-client".to_string(),
                redirect_uri: "https://app.lagrange.local/auth/callback".to_string(),
                authorize_url: "https://issuer.test/authorize".to_string(),
                token_url: "https://issuer.test/oauth/token".to_string(),
                jwks_url: "https://issuer.test/.well-known/jwks.json".to_string(),
                audience: Some(SIM_AUDIENCE.to_string()),
                clock_skew_secs: 60,
            },
            transport: callback_simulator.clone(),
        },
        auth::invites::InviteService::new(
            std::sync::Arc::new(callback_invites.clone()),
            std::sync::Arc::new(callback_users.clone()),
            std::sync::Arc::new(FakeClock(expires_at - 3_200)),
            callback_audit.clone(),
        ),
        SessionService::new(
            std::sync::Arc::new(PostgresSessionStore::new(app.clone(), admin.clone())),
            std::sync::Arc::new(FakeClock(expires_at - 3_200)),
            callback_audit,
        ),
        std::sync::Arc::new(NoopAudit),
    );
    let pending_store = InMemoryPendingAuthStore::default();
    let begin = callback_auth.begin_login()?;
    let state = begin.state.clone();
    let nonce = begin.nonce.clone();
    pending_store
        .insert(
            state.clone(),
            PendingAuth {
                state: state.clone(),
                nonce: nonce.clone(),
                code_verifier: begin.pkce.verifier.clone(),
                created_at_secs: expires_at - 3_200,
                ttl_secs: 300,
            },
        )
        .await?;
    let callback_claims = |nonce: &str| {
        serde_json::json!({
            "iss": "https://issuer.test",
            "sub": "auth0|callback-member",
            "aud": [SIM_AUDIENCE],
            "exp": expires_at,
            "iat": expires_at - 3_200,
            "nonce": nonce,
            "email": "callback-member@example.test",
            "email_verified": true,
            "auth_time": expires_at - 3_260,
            "amr": ["pwd"],
            "roles": ["member"],
        })
    };
    let code = callback_simulator.issue_code(callback_claims(&nonce), &begin.pkce.verifier);
    let first_callback = callback_auth
        .complete_login(&code, &state, &pending_store)
        .await?;
    assert!(Uuid::parse_str(&first_callback.session.user_id.0).is_ok());
    assert_eq!(
        callback_auth
            .session_info(&first_callback.cookie_value)
            .await?
            .user_id,
        first_callback.session.user_id
    );
    let replay = callback_auth
        .complete_login(&code, &state, &pending_store)
        .await
        .expect_err("consumed callback state must not replay");
    assert_eq!(replay.code(), "STATE_MISMATCH");

    let retry_begin = callback_auth.begin_login()?;
    let retry_state = retry_begin.state.clone();
    let retry_nonce = retry_begin.nonce.clone();
    pending_store
        .insert(
            retry_state.clone(),
            PendingAuth {
                state: retry_state.clone(),
                nonce: retry_nonce.clone(),
                code_verifier: retry_begin.pkce.verifier.clone(),
                created_at_secs: expires_at - 3_100,
                ttl_secs: 300,
            },
        )
        .await?;
    let retry_code =
        callback_simulator.issue_code(callback_claims(&retry_nonce), &retry_begin.pkce.verifier);
    let retry_callback = callback_auth
        .complete_login(&retry_code, &retry_state, &pending_store)
        .await?;
    assert_eq!(
        retry_callback.session.user_id, first_callback.session.user_id,
        "established issuer/subject must retain its canonical UUID"
    );

    let app_first = app.clone();
    let app_second = app.clone();
    let first = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4)")
            .bind(owner_id)
            .bind(invitation_id)
            .bind("https://issuer.test")
            .bind("auth0|concurrent-member")
            .fetch_one(&app_first)
            .await
    };
    let second = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4)")
            .bind(owner_id)
            .bind(invitation_id)
            .bind("https://issuer.test")
            .bind("auth0|concurrent-member")
            .fetch_one(&app_second)
            .await
    };
    let (first, second) = tokio::join!(first, second);
    let claimed = [first?, second?];
    assert!(claimed.iter().all(|claimed| *claimed));

    let redeemed: (String, Option<Uuid>, String) = sqlx::query_as(
        "SELECT status, redeemed_by_user_id, role_id FROM invitations WHERE id = $1",
    )
    .bind(invitation_id)
    .fetch_one(&admin)
    .await?;
    assert_eq!(redeemed.0, "REDEEMED");
    assert!(redeemed.1.is_some());
    assert_eq!(redeemed.2, "member");
    assert!(
        adapter_users
            .find_by_binding("https://issuer.test", "auth0|concurrent-member")
            .await?
            .is_none(),
        "provisional identities must not be treated as established sessions"
    );
    let provisional_invite = PostgresInviteStore::new(app.clone(), admin.clone())
        .find_by_email("new-member@example.test")
        .await?
        .expect("redeemed provisional invitation remains retryable");
    assert_eq!(provisional_invite.id, invitation_id.to_string());
    let user_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE issuer = 'https://issuer.test' \
         AND subject = 'auth0|concurrent-member'",
    )
    .fetch_one(&admin)
    .await?;
    assert_eq!(user_count, 1);

    let bound_id: Uuid = sqlx::query_scalar("SELECT public.bind_redeemed_identity($1, $2, $3, $4)")
        .bind("https://issuer.test")
        .bind("auth0|concurrent-member")
        .bind("new-member@example.test")
        .bind("member")
        .fetch_one(&app)
        .await?;
    assert_eq!(bound_id, redeemed.1.expect("redeemed user"));
    assert!(
        adapter_users
            .find_by_binding("https://issuer.test", "auth0|concurrent-member")
            .await?
            .is_some(),
        "finalized identities must be visible to the serving adapter"
    );
    let provenance: Option<Uuid> =
        sqlx::query_scalar("SELECT provisioned_by_user_id FROM users WHERE id = $1")
            .bind(bound_id)
            .fetch_one(&admin)
            .await?;
    assert!(provenance.is_none());
    let repeat_bind = sqlx::query("SELECT public.bind_redeemed_identity($1, $2, $3, $4)")
        .bind("https://issuer.test")
        .bind("auth0|concurrent-member")
        .bind("new-member@example.test")
        .bind("member")
        .fetch_one(&app)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&repeat_bind).as_deref(), Some("42501"));

    let preprovisioned_bind = sqlx::query("SELECT public.bind_redeemed_identity($1, $2, $3, $4)")
        .bind("https://issuer.test")
        .bind("provision-owner")
        .bind("provision-owner@example.test")
        .bind("owner")
        .fetch_one(&app)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&preprovisioned_bind).as_deref(), Some("42501"));

    for (role, expected) in [
        ("app", true),
        ("admin", false),
        ("worker", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        for signature in [
            "public.create_invitation(uuid, text, text, text, bigint)",
            "public.claim_invitation(uuid, uuid, text, text)",
            "public.bind_redeemed_identity(text, text, text, text)",
        ] {
            let can_execute: bool =
                sqlx::query_scalar("SELECT has_function_privilege($1, $2, 'EXECUTE')")
                    .bind(role)
                    .bind(signature)
                    .fetch_one(owner)
                    .await?;
            assert_eq!(
                can_execute, expected,
                "unexpected {role} grant on {signature}"
            );
        }
    }
    let function_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.claim_invitation(uuid,uuid,text,text)'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(function_metadata.0);
    assert_eq!(function_metadata.1, "migration_owner");
    let function_config = function_metadata.2.expect("claim function config");
    assert!(
        function_config
            .iter()
            .any(|config| config == "search_path=pg_catalog, pg_temp")
    );
    assert!(
        function_config
            .iter()
            .any(|config| config == "lock_timeout=1s")
    );
    assert!(
        function_config
            .iter()
            .any(|config| config == "statement_timeout=5s")
    );

    let blocked_rollback = MIGRATOR.undo(owner, 38).await.unwrap_err();
    assert_eq!(migrate_pg_code(&blocked_rollback).as_deref(), Some("55000"));
    sqlx::query("DELETE FROM invitations WHERE id = $1")
        .bind(owner_invitation_id)
        .execute(owner)
        .await?;
    MIGRATOR.undo(owner, 38).await?;
    let function_exists: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('public.claim_invitation(uuid,uuid,text,text)') IS NOT NULL",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        !function_exists,
        "0040 down must remove provisioning functions"
    );
    MIGRATOR.run(owner).await?;
    Ok(())
}

#[test]
fn recommendation_pipeline_migration_is_tracked() {
    for migration in [
        RECOMMENDATION_PIPELINE_UP_SQL,
        RECOMMENDATION_PIPELINE_DOWN_SQL,
    ] {
        assert!(migration.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(migration.contains("SET LOCAL statement_timeout = '30s'"));
    }
    for version in 26..=38 {
        let up = MIGRATOR.migrations.iter().find(|migration| {
            migration.version == version
                && migration.migration_type != MigrationType::ReversibleDown
        });
        let down = MIGRATOR.migrations.iter().find(|migration| {
            migration.version == version
                && migration.migration_type == MigrationType::ReversibleDown
        });
        assert!(up.is_some(), "migration {version:04} up must exist");
        assert!(down.is_some(), "migration {version:04} down must exist");

        let expected_no_tx = matches!(version, 27 | 28 | 29 | 31 | 32);
        for migration in [up.unwrap(), down.unwrap()] {
            assert_eq!(
                migration.no_tx, expected_no_tx,
                "migration {version:04} transaction mode is wrong"
            );
            if expected_no_tx {
                assert_eq!(
                    executable_sql(migration.sql.as_str())
                        .matches("CONCURRENTLY")
                        .count(),
                    1,
                    "each no-transaction migration must contain exactly one concurrent statement"
                );
            }
        }
    }
    for migration in [
        RECOMMENDATION_ITEM_CONSTRAINT_UP_SQL,
        RECOMMENDATION_ITEM_CONSTRAINT_DOWN_SQL,
    ] {
        assert!(migration.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(migration.contains("SET LOCAL statement_timeout = '30s'"));
    }
    assert!(RECOMMENDATION_PIPELINE_UP_SQL.contains("recommendation_scheduler_control"));
    assert!(
        !RECOMMENDATION_PIPELINE_UP_SQL
            .contains("GRANT EXECUTE ON FUNCTION\n    public.schedule_recommendation_run")
    );
    assert!(RECOMMENDATION_ROLLBACK_GUARD_UP_SQL.contains("active = true"));
    assert!(RECOMMENDATION_ROLLBACK_GUARD_UP_SQL.contains("GRANT EXECUTE ON FUNCTION"));
    assert!(RECOMMENDATION_ROLLBACK_GUARD_DOWN_SQL.contains("pg_advisory_xact_lock"));
    assert!(RECOMMENDATION_ROLLBACK_GUARD_DOWN_SQL.contains("active = false"));
    assert!(RECOMMENDATION_ROLLBACK_GUARD_DOWN_SQL.contains("REVOKE EXECUTE ON FUNCTION"));
    assert!(RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("SECURITY DEFINER"));
    assert!(
        RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("SET search_path = pg_catalog, pg_temp")
    );
    assert!(RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("FROM public.user_strategy_configs"));
    assert!(RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("FROM public.dataset_versions"));
    assert!(RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("FROM public.universe_snapshots"));
    assert_eq!(
        RECOMMENDATION_PUBLICATION_LOCK_UP_SQL
            .matches("FOR SHARE")
            .count(),
        3
    );
    assert!(RECOMMENDATION_PUBLICATION_LOCK_UP_SQL.contains("GRANT EXECUTE"));
    assert!(RECOMMENDATION_PUBLICATION_LOCK_DOWN_SQL.contains("REVOKE EXECUTE"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("SECURITY DEFINER"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("FOR SHARE"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("covered_uses"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("lock_recommendation_source_pins"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("p_source_file_names"));
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("p_content_sha256s"));
    assert!(
        RECOMMENDATION_ENTITLEMENT_LOCK_UP_SQL.contains("jobs_sync_recommendation_terminal_run")
    );
    assert!(RECOMMENDATION_ENTITLEMENT_LOCK_DOWN_SQL.contains("DROP TRIGGER"));
    for migration in [
        RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL,
        RECOMMENDATION_SUBMISSION_DATASET_LOCK_DOWN_SQL,
    ] {
        assert!(migration.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(migration.contains("SET LOCAL statement_timeout = '30s'"));
    }
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL.contains("SECURITY DEFINER"));
    assert!(
        RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL
            .contains("SET search_path = pg_catalog, pg_temp")
    );
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL.contains("status = 'READY'"));
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL.contains("FOR SHARE OF dataset"));
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL.contains("FROM PUBLIC"));
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_UP_SQL.contains("TO app"));
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_DOWN_SQL.contains("FROM app"));
    assert!(RECOMMENDATION_SUBMISSION_DATASET_LOCK_DOWN_SQL.contains("DROP FUNCTION"));
    for migration in [
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL,
        PAPER_RECOMMENDATION_EXECUTION_DOWN_SQL,
    ] {
        assert!(migration.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(migration.contains("SET LOCAL statement_timeout = '30s'"));
    }
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("dataset_version_id uuid"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("dataset_manifest_sha256 text"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("non_execution_reason jsonb"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("non_execution_reason ? 'code'"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("non_execution_reason ? 'message'"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("item ->> 'weight' IS NULL"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("lock_recommendation_schedule_inputs"));
    assert!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("lock_recommendation_calendar_coverage")
    );
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("preflight_paper_target"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("queue_scheduled_paper_targets"));
    assert_eq!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL
            .matches("dataset.id = p_dataset_version_id")
            .count(),
        2,
        "both scheduling and Paper queue boundaries must attest the exact dataset UUID"
    );
    assert_eq!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL
            .matches("dataset.version = p_dataset_version")
            .count(),
        2,
        "both scheduling and Paper queue boundaries must attest the exact dataset version"
    );
    assert_eq!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL
            .matches("SECURITY DEFINER")
            .count(),
        4
    );
    assert!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("SET search_path = pg_catalog, pg_temp")
    );
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("FOR SHARE"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("FOR UPDATE"));
    assert!(PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("TO worker"));
    assert!(!PAPER_RECOMMENDATION_EXECUTION_UP_SQL.contains("TO app"));
    assert!(
        PAPER_RECOMMENDATION_EXECUTION_UP_SQL
            .contains("REVOKE INSERT ON TABLE public.pending_targets FROM worker")
    );
    assert!(PAPER_RECOMMENDATION_EXECUTION_DOWN_SQL.contains("FROM worker"));
    assert!(
        PAPER_RECOMMENDATION_EXECUTION_DOWN_SQL
            .contains("GRANT INSERT ON TABLE public.pending_targets TO worker")
    );
    assert!(PAPER_RECOMMENDATION_EXECUTION_DOWN_SQL.contains("DROP FUNCTION"));

    for migration in [
        PAPER_REBALANCE_PREVIEW_UP_SQL,
        PAPER_REBALANCE_PREVIEW_DOWN_SQL,
    ] {
        assert!(migration.contains("SET LOCAL lock_timeout = '5s'"));
        assert!(migration.contains("SET LOCAL statement_timeout = '30s'"));
    }
    for token in [
        "paper_rebalance_previews",
        "paper_state_version",
        "lock_paper_rebalance_preview_submission",
        "snapshot_paper_rebalance_preview",
        "publish_paper_rebalance_preview",
        "fail_paper_rebalance_preview",
        "apply_paper_rebalance_preview",
        "MANUAL_RECOMMENDATION",
        "SCHEDULED_RECOMMENDATION",
        "SECURITY DEFINER",
        "SET search_path = pg_catalog, pg_temp",
    ] {
        assert!(
            PAPER_REBALANCE_PREVIEW_UP_SQL.contains(token),
            "0038 up is missing {token}"
        );
    }
    assert!(PAPER_REBALANCE_PREVIEW_DOWN_SQL.contains("preview rollback blocked"));
    assert!(PAPER_REBALANCE_PREVIEW_DOWN_SQL.contains("DROP TABLE"));
}

#[test]
fn tracked_research_schema_gate_is_fail_closed_and_migrations_bound_locks() {
    for token in [
        "version IN (22, 23, 24, 25, 33, 34, 35)",
        "convalidated",
        "pg_get_constraintdef",
        "format_type",
        "attnotnull",
        "attidentity",
        "pg_get_expr",
        "storage_path",
        "EXCEPT",
        "indisunique",
        "indisvalid",
        "indisready",
        "indislive",
        "relrowsecurity",
        "rolcanlogin",
        "rolsuper",
        "rolbypassrls",
        "rolcreatedb",
        "rolcreaterole",
        "pg_auth_members",
        "polcmd",
        "polpermissive",
        "tgenabled",
        "tgtype",
        "prosecdef",
        "pg_get_functiondef",
        "regexp_replace",
        "actual_function",
        "expected_function",
        "role_table_grants",
        "has_schema_privilege",
        "has_table_privilege",
        "has_sequence_privilege",
        "lock_recommendation_source_pins",
    ] {
        assert!(
            RESEARCH_SCHEMA_GATE_SQL.contains(token),
            "tracked research schema gate is missing {token}"
        );
    }
    for migration in [SOURCE_INDEX_UP_SQL, CALENDAR_VERSION_INDEX_UP_SQL] {
        assert!(migration.contains("PGOPTIONS='-c lock_timeout=5s' sqlx migrate run"));
        assert!(migration.contains("CONCURRENTLY"));
    }
}

#[tokio::test]
async fn research_schema_gate_accepts_current_and_future_migration_ledgers() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = async {
        MIGRATOR.run(&owner).await?;
        sqlx::raw_sql(RESEARCH_SCHEMA_GATE_SQL)
            .execute(&owner)
            .await?;
        let future_version = up_migration_count() as i64 + 1;
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES ($1, 'future migration', now(), true, decode(repeat('00', 32), 'hex'), 0)",
        )
        .bind(future_version)
        .execute(&owner)
        .await?;
        sqlx::raw_sql(RESEARCH_SCHEMA_GATE_SQL)
            .execute(&owner)
            .await?;
        sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = 33")
            .execute(&owner)
            .await?;
        let missing_required = sqlx::raw_sql(RESEARCH_SCHEMA_GATE_SQL)
            .execute(&owner)
            .await
            .unwrap_err();
        assert!(
            missing_required
                .to_string()
                .contains("successful SQLx migrations 22-25 and 33-35 are required")
        );
        sqlx::query("UPDATE _sqlx_migrations SET success = true WHERE version = 33")
            .execute(&owner)
            .await?;
        sqlx::raw_sql(RESEARCH_SCHEMA_GATE_SQL)
            .execute(&owner)
            .await?;
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("research schema gate ledger contract FAILED: {error}");
    }
}

fn executable_sql(sql: &str) -> String {
    sql.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Database-local grants executed as supervisor on each fresh scratch database.
const BOOTSTRAP_SQL: &str = include_str!("../bootstrap.sql");
/// Cluster-global roles serialized in the supervisor database before scratch creation.
const ROLE_BOOTSTRAP_SQL: &str = include_str!("../role-bootstrap.sql");

/// Tenant tables that MUST carry an ownership column (design §7.3).
const TENANT_TABLES: &[&str] = &[
    "user_strategy_configs",
    "recommendation_runs",
    "recommendation_items",
    "target_portfolios",
    "paper_rebalance_previews",
    "jobs",
    "backtest_runs",
    "backtest_metrics",
    "backtest_warnings",
    "result_artifacts",
    "accounts",
    "cash_ledger",
    "positions",
    "orders",
    "fills",
    "daily_equity",
    "broker_connections",
    "reconciliation_runs",
    "risk_events",
    "notifications",
    "web_sessions",
    "invitations",
];

const PUBLIC_JOB_STATUSES: [&str; 5] = ["QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"];
const RECOMMENDATION_FENCE_LOCK_CLASS: i32 = 1_815_099_521;
const RECOMMENDATION_FENCE_LOCK_OBJECT: i32 = 33;

fn pg_code(err: &sqlx::Error) -> Option<String> {
    match err {
        // sqlx 0.9's `DatabaseError::code()` returns `Option<Cow<'_, str>>`;
        // materialize an owned String so the error code outlives `err`.
        sqlx::Error::Database(e) => e.code().map(|c| c.into_owned()),
        _ => None,
    }
}

fn migrate_pg_code(err: &sqlx::migrate::MigrateError) -> Option<String> {
    match err {
        sqlx::migrate::MigrateError::Execute(error)
        | sqlx::migrate::MigrateError::ExecuteMigration(error, _) => pg_code(error),
        _ => None,
    }
}

async fn wait_for_advisory_wait(
    observer: &PgPool,
    backend_pid: i32,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..200 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
               SELECT 1 FROM pg_stat_activity \
               WHERE pid = $1 AND wait_event_type = 'Lock' AND wait_event = 'advisory')",
        )
        .bind(backend_pid)
        .fetch_one(observer)
        .await?;
        if waiting {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    type Activity = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let activity: Option<Activity> = sqlx::query_as(
        "SELECT state, wait_event_type, wait_event, query \
         FROM pg_stat_activity WHERE pid = $1",
    )
    .bind(backend_pid)
    .fetch_optional(observer)
    .await?;
    Err(
        format!("{label} backend {backend_pid} did not wait on the advisory fence: {activity:?}")
            .into(),
    )
}

/// Audit point for dynamic DDL. `CREATE/DROP DATABASE` and
/// `GRANT CONNECT ON DATABASE` take database *identifiers*, which PostgreSQL
/// cannot express as bind parameters, so these statements must be assembled
/// dynamically. Injection is impossible: the only interpolated value is the
/// database name produced by `fresh_db_name()` (`contract_{pid}_{ts}`) and
/// asserted to contain only `[a-z0-9_]` in `create_contract_db` before any
/// statement is built. `AssertSqlSafe` records that audit for sqlx 0.9's
/// compile-time SQL audit.
fn ddl_for(db: &str, statement: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(statement.replace("{db}", db))
}

/// Number of rows in sqlx's bookkeeping table — the count of applied
/// migrations. sqlx 0.9's `Migrator::run`/`undo` return `()` (0.8 returned a
/// count), so the contract asserts on this table instead.
async fn applied_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
}

#[test]
fn cluster_role_bootstrap_cannot_grant_scratch_schema_privileges() {
    let executable = executable_sql(ROLE_BOOTSTRAP_SQL).to_ascii_uppercase();
    assert!(!executable.contains("GRANT "));
    assert!(!executable.contains("SCHEMA "));
    let scratch_executable = executable_sql(BOOTSTRAP_SQL).to_ascii_uppercase();
    assert!(!scratch_executable.contains("CREATE ROLE"));
    assert!(!scratch_executable.contains("ALTER ROLE"));
}

/// Rewrite a supervisor URL (`postgres://user[:pw]@host:port/db`) for the
/// legacy serving-role checks. Migration and research writer pools retain the
/// supplied supervisor identity and assume their fixed effective role in
/// `after_connect` instead.
fn conn_url(super_url: &str, role: &str, db: &str) -> String {
    let (_scheme, rest) = super_url
        .split_once("://")
        .expect("DATABASE_URL must start with a scheme");
    let (auth, hostport_db) = rest.split_once('@').expect("DATABASE_URL must contain @");
    let (user, pw) = match auth.split_once(':') {
        Some((u, p)) => (u, Some(p)),
        None => (auth, None),
    };
    let (hostport, _old_db) = hostport_db
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    let _ = user; // role replaces the original user
    match pw {
        Some(p) => format!("postgres://{role}:{p}@{hostport}/{db}"),
        None => format!("postgres://{role}@{hostport}/{db}"),
    }
}

fn supervisor_db_url(super_url: &str, db: &str) -> String {
    let (head, _) = super_url
        .rsplit_once('/')
        .expect("DATABASE_URL must contain a database path");
    format!("{head}/{db}")
}

async fn effective_role_pool(
    super_url: &str,
    db: &str,
    role: &'static str,
    actor_user_id: Option<&str>,
    max_connections: u32,
) -> Result<PgPool, Box<dyn Error>> {
    let setup = match role {
        "migration_owner" => "SET ROLE migration_owner",
        "research_writer" => "SET ROLE research_writer",
        _ => return Err(format!("unsupported effective role {role}").into()),
    };
    let mut options: sqlx::postgres::PgConnectOptions = supervisor_db_url(super_url, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    if let Some(user_id) = actor_user_id {
        options = options.options([("app.actor_user_id", user_id.to_owned())]);
    }
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::raw_sql(setup).execute(&mut *connection).await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .map_err(Box::<dyn Error>::from)?;
    let identities: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&pool)
        .await?;
    assert_eq!(identities.0, role);
    assert_ne!(identities.0, identities.1);
    Ok(pool)
}

/// A pool whose connections carry an explicit RLS actor context
/// (`app.actor_user_id` startup option). Since migration 0010 forces row-level
/// security on every tenant table, tenant reads (which are STRICT: the policy
/// requires `owner = current_setting('app.actor_user_id')`) only return rows
/// when the connection carries the actor GUC. The migration owner may touch
/// tenant rows while impersonating a user; without the GUC, FORCE RLS denies
/// it (hazard proven by the Todo 23 tenancy suite).
async fn actor_pool(
    super_url: &str,
    db: &str,
    role: &str,
    user_id: &str,
) -> Result<PgPool, Box<dyn Error>> {
    if role == "migration_owner" {
        return effective_role_pool(super_url, db, "migration_owner", Some(user_id), 3).await;
    }
    let opts: sqlx::postgres::PgConnectOptions = conn_url(super_url, role, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    PgPoolOptions::new()
        .max_connections(3)
        .connect_with(opts.options([("app.actor_user_id", user_id.to_string())]))
        .await
        .map_err(Box::<dyn Error>::from)
}

/// Scratch-database name, unique per call. Both tests share one process and
/// run in PARALLEL test threads, so pid+millis alone can collide (two
/// `CREATE DATABASE` of the same name -> duplicate key on
/// `pg_database_datname_index`); the monotonic counter guarantees uniqueness.
fn fresh_db_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis();
    format!(
        "contract_{}_{}_{}",
        std::process::id(),
        ts,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Number of migrations `Migrator::run` applies. sqlx 0.9 records each
/// `.up.sql` and `.down.sql` file as its OWN `Migration` entry
/// (`ReversibleUp`/`ReversibleDown`), so `migrations.len()` is 2x the real
/// count for the reversible migration pairs in `migrations/`. `run` applies
/// every non-`ReversibleDown` entry; `undo` consumes the down side.
fn up_migration_count() -> usize {
    MIGRATOR
        .migrations
        .iter()
        .filter(|m| m.migration_type != MigrationType::ReversibleDown)
        .count()
}

async fn admin_pool(url: &str) -> Result<PgPool, Box<dyn Error>> {
    Ok(PgPoolOptions::new().max_connections(3).connect(url).await?)
}

/// Create a brand-new database on the disposable cluster, bootstrap roles and
/// schema grants, and return `(db_name, migration_owner_pool)`.
async fn create_contract_db(super_url: &str) -> Result<(String, PgPool), Box<dyn Error>> {
    let db = fresh_db_name();
    assert!(
        db.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated database name must be a safe identifier"
    );
    let admin = admin_pool(super_url).await?;
    let mut role_bootstrap = admin.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('lagrange-test-role-bootstrap'))")
        .execute(&mut *role_bootstrap)
        .await?;
    sqlx::raw_sql(ROLE_BOOTSTRAP_SQL)
        .execute(&mut *role_bootstrap)
        .await?;
    role_bootstrap.commit().await?;
    sqlx::query(ddl_for(&db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    sqlx::query(ddl_for(&db, "CREATE DATABASE {db}"))
        .execute(&admin)
        .await?;
    drop(admin);

    let super_new = admin_pool(&supervisor_db_url(super_url, &db)).await?;
    sqlx::raw_sql(BOOTSTRAP_SQL).execute(&super_new).await?;
    sqlx::raw_sql(ddl_for(
        &db,
        "GRANT CONNECT ON DATABASE {db} TO migration_owner, app, worker, audit_writer, research_writer, admin",
    ))
    .execute(&super_new)
    .await?;
    drop(super_new);

    let owner = effective_role_pool(super_url, &db, "migration_owner", None, 3).await?;
    let roles: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&owner)
        .await?;
    assert_eq!(roles.0, "migration_owner");
    assert_ne!(roles.0, roles.1);
    Ok((db, owner))
}

/// Drop the disposable database, terminating any remaining connections.
async fn drop_contract_db(super_url: &str, db: &str) -> Result<(), Box<dyn Error>> {
    let admin = admin_pool(super_url).await?;
    sqlx::query(ddl_for(db, "DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .execute(&admin)
        .await?;
    Ok(())
}

async fn role_pool(super_url: &str, db: &str, role: &str) -> Result<PgPool, Box<dyn Error>> {
    if role == "research_writer" {
        return effective_role_pool(super_url, db, "research_writer", None, 2).await;
    }
    let opts: sqlx::postgres::PgConnectOptions = conn_url(super_url, role, db)
        .parse()
        .map_err(Box::<dyn Error>::from)?;
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await?)
}

fn require_db_url() -> Result<String, Box<dyn Error>> {
    match env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()) {
        Some(url) => Ok(url),
        None => {
            eprintln!("SKIP: DATABASE_URL not set - no disposable PostgreSQL cluster available");
            Err("DATABASE_URL not set".into())
        }
    }
}

/// Full contract: migrate run -> no-op re-run -> five-state CHECK -> ORPHANED
/// attempt -> ownership columns -> role invariants -> app-role denials ->
/// audit append-only -> worker capability -> hash/unique/check constraints.
///
/// Un-gated since Todo 3's live gate passed: runs with `cargo test -p
/// migration-contract` against `DATABASE_URL` (disposable scratch DB).
#[tokio::test]
async fn migration_contract_full() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };

    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = full_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await; // best-effort cleanup
    if let Err(e) = result {
        panic!("migration contract FAILED: {e}");
    }
}

async fn full_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    // ------------------------------------------------------------------
    // 1. `sqlx migrate run` applies every migration; a second run is a no-op.
    // ------------------------------------------------------------------
    assert!(
        RESEARCH_PUBLICATION_DOWN_SQL.contains("SET LOCAL lock_timeout = '5s';"),
        "0022 down must bound blocking rollback DDL with a transactional lock_timeout"
    );
    for (name, sql, expected_statement) in [
        (
            "0024 up",
            SOURCE_INDEX_UP_SQL,
            "CREATE UNIQUE INDEX CONCURRENTLY data_batches_source_file_uq\nON data_batches (provider, market, source_batch_id, source_file_name)\nWHERE source_batch_id IS NOT NULL;",
        ),
        (
            "0024 down",
            SOURCE_INDEX_DOWN_SQL,
            "DROP INDEX CONCURRENTLY IF EXISTS data_batches_source_file_uq;",
        ),
        (
            "0025 up",
            CALENDAR_VERSION_INDEX_UP_SQL,
            "CREATE INDEX CONCURRENTLY trading_calendar_versions_source_lookup_idx\nON trading_calendar_versions (exchange, source_version)\nINCLUDE (source, timezone, content_sha256);",
        ),
        (
            "0025 down",
            CALENDAR_VERSION_INDEX_DOWN_SQL,
            "DROP INDEX CONCURRENTLY IF EXISTS trading_calendar_versions_source_lookup_idx;",
        ),
    ] {
        assert!(
            sql.starts_with("-- no-transaction"),
            "{name} must begin with SQLx's no-transaction directive"
        );
        assert!(
            sql.contains("externally") && sql.contains("lock_timeout"),
            "{name} must document externally supplied finite lock_timeout"
        );
        assert_eq!(
            executable_sql(sql),
            expected_statement,
            "{name} must contain only the concurrent DDL statement"
        );
    }
    let expected = up_migration_count();
    assert!(expected > 0, "migrator must embed at least one migration");
    MIGRATOR.run(owner).await?;
    let applied = applied_count(owner).await? as usize;
    assert_eq!(
        applied, expected,
        "first run must apply all {expected} migrations"
    );
    MIGRATOR.run(owner).await?;
    let applied_again = applied_count(owner).await? as usize;
    assert_eq!(applied_again, applied, "second run must be a no-op");

    let source_index_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 24 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0024 source-lineage index migration must exist");
    assert!(
        source_index_migration.no_tx,
        "0024 must opt out of a transaction so PostgreSQL can build its index concurrently"
    );
    let source_index_down_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 24 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0024 source-lineage index down migration must exist");
    assert!(
        source_index_down_migration.no_tx,
        "0024 down must opt out of a transaction so PostgreSQL can drop its index concurrently"
    );
    let calendar_index_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 25 && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("0025 calendar source-version lookup migration must exist");
    assert!(
        calendar_index_migration.no_tx,
        "0025 must opt out of a transaction so PostgreSQL can build its index concurrently"
    );
    let calendar_index_down_migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == 25 && migration.migration_type == MigrationType::ReversibleDown
        })
        .expect("0025 calendar source-version lookup down migration must exist");
    assert!(
        calendar_index_down_migration.no_tx,
        "0025 down must opt out of a transaction so PostgreSQL can drop its index concurrently"
    );
    let calendar_index_shape: (i32, i32, bool, bool) = sqlx::query_as(
        "SELECT indnkeyatts::integer, indnatts::integer, indisunique, indisvalid \
         FROM pg_index WHERE indexrelid = \
         'public.trading_calendar_versions_source_lookup_idx'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(calendar_index_shape, (2, 5, false, true));
    let calendar_index_definition: String = sqlx::query_scalar(
        "SELECT pg_get_indexdef('public.trading_calendar_versions_source_lookup_idx'::regclass)",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_index_definition.contains(
            "USING btree (exchange, source_version) INCLUDE (source, timezone, content_sha256)"
        ),
        "unexpected 0025 index definition: {calendar_index_definition}"
    );

    // Tables exist.
    let jobs_class: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.jobs')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        jobs_class.as_deref(),
        Some("jobs"),
        "public.jobs must exist"
    );
    let attempts_class: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.job_attempts')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        attempts_class.as_deref(),
        Some("job_attempts"),
        "public.job_attempts must exist"
    );

    // ------------------------------------------------------------------
    // 2. Ownership columns on every tenant table (design §7.3).
    // ------------------------------------------------------------------
    for t in TENANT_TABLES {
        let cols: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1",
        )
        .bind(t)
        .fetch_all(owner)
        .await?;
        let has_ownership = cols.iter().any(|c| {
            matches!(
                c.as_str(),
                "owner_user_id" | "account_id" | "user_id" | "created_by_user_id"
            )
        });
        assert!(
            has_ownership,
            "tenant table `{t}` must carry an ownership column, got columns {cols:?}"
        );
    }

    // ------------------------------------------------------------------
    // 3. Role invariants: every table owned by migration_owner; serving
    //    roles have no BYPASSRLS.
    // ------------------------------------------------------------------
    let owners: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT tableowner FROM pg_tables WHERE schemaname = 'public'",
    )
    .fetch_all(owner)
    .await?
    .into_iter()
    .collect();
    assert!(!owners.is_empty(), "at least one table must exist");
    assert!(
        owners.iter().all(|o| o == "migration_owner"),
        "all tables must be owned by migration_owner, got {owners:?}"
    );
    for role in ["app", "worker", "audit_writer", "research_writer", "admin"] {
        let bypass: bool =
            sqlx::query_scalar::<_, bool>("SELECT rolbypassrls FROM pg_roles WHERE rolname = $1")
                .bind(role)
                .fetch_one(owner)
                .await?;
        assert!(!bypass, "role {role} must not have BYPASSRLS");
    }

    // ------------------------------------------------------------------
    // 4. jobs.status: EXACTLY five public values; ORPHANED never a sixth.
    // ------------------------------------------------------------------
    let uid: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (issuer, subject, email, display_name) \
         VALUES ('https://issuer.test','owner-subject','owner@example.test','Owner') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    // Tenant DML runs under an explicit actor context (RLS policies are
    // strict for writes too since migration 0010).
    let owner_actor = actor_pool(super_url, db, "migration_owner", &uid.to_string()).await?;
    for (i, status) in PUBLIC_JOB_STATUSES.iter().enumerate() {
        sqlx::query(
            "INSERT INTO jobs (owner_user_id, job_type, status, priority, payload_json, \
             max_attempts, idempotency_key) VALUES ($1, 'backtest', $2, 10, '{}'::jsonb, 3, $3)",
        )
        .bind(uid)
        .bind(status)
        .bind(format!("idem-{i}"))
        .execute(&owner_actor)
        .await?;
    }
    let sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&sixth).as_deref(),
        Some("23514"),
        "a sixth public jobs.status (ORPHANED) must be rejected by CHECK"
    );
    let checkdef: String = sqlx::query_scalar::<_, String>(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conrelid = 'public.jobs'::regclass AND conname = 'jobs_status_check'",
    )
    .fetch_one(owner)
    .await?;
    for s in PUBLIC_JOB_STATUSES {
        assert!(
            checkdef.contains(s),
            "jobs_status_check must list {s}, got {checkdef}"
        );
    }
    assert!(
        !checkdef.contains("ORPHANED"),
        "ORPHANED must never appear in the public jobs.status CHECK, got {checkdef}"
    );

    // ------------------------------------------------------------------
    // 5. job_attempts.outcome includes ORPHANED (attempt-level only).
    // ------------------------------------------------------------------
    let job_id: Uuid =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM jobs ORDER BY created_at LIMIT 1")
            .fetch_one(&owner_actor)
            .await?;
    sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome, claimed_by) \
         VALUES ($1, 1, 'ORPHANED', 'worker-probe')",
    )
    .bind(job_id)
    .execute(owner)
    .await?;
    let canceled_attempt = sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome) VALUES ($1, 2, 'CANCELED')",
    )
    .bind(job_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&canceled_attempt).as_deref(),
        Some("23514"),
        "CANCELED is not an attempt-level outcome; ORPHANED covers worker death"
    );
    let dup_attempt = sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome) VALUES ($1, 1, 'RUNNING')",
    )
    .bind(job_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&dup_attempt).as_deref(),
        Some("23505"),
        "attempt_no must be unique per job"
    );

    // ------------------------------------------------------------------
    // 6. App role: functional on tenant data, denied everything else.
    // ------------------------------------------------------------------
    let app = role_pool(super_url, db, "app").await?;
    // Tenant writes that RETURN rows run under an explicit actor context
    // (Todo 23 RLS policies are strict when a GUC is present); the app pool
    // acts "as the owner" for the positive tenant assertions.
    let app_actor = actor_pool(super_url, db, "app", &uid.to_string()).await?;
    // Positive: app serves its owner's tenant data.
    let acc: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency) \
         VALUES ($1, 'PAPER', 'qa-paper', 'KRW') RETURNING id",
    )
    .bind(uid)
    .fetch_one(&app_actor)
    .await?;
    assert!(
        acc.as_bytes().len() == 16,
        "app must be able to insert tenant rows"
    );

    // Denial: ALTER TABLE (no ownership, no schema CREATE).
    let ddl = sqlx::query("ALTER TABLE jobs ADD COLUMN hacked_by_app text")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&ddl).as_deref(),
        Some("42501"),
        "app role must not ALTER TABLE"
    );
    let create_table = sqlx::query("CREATE TABLE app_hack (id integer)")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&create_table).as_deref(),
        Some("42501"),
        "app role must not CREATE TABLE"
    );

    // Denial: TRUNCATE audit_logs.
    let truncate = sqlx::query("TRUNCATE TABLE audit_logs")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&truncate).as_deref(),
        Some("42501"),
        "app role must not TRUNCATE audit_logs"
    );

    // Denial: audit rows are append-only for app (SELECT only).
    let upd = sqlx::query("UPDATE audit_logs SET reason = 'tampered'")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&upd).as_deref(),
        Some("42501"),
        "app role must not UPDATE audit_logs"
    );
    let del = sqlx::query("DELETE FROM audit_logs")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&del).as_deref(),
        Some("42501"),
        "app role must not DELETE audit_logs"
    );
    let app_audit_insert = sqlx::query("INSERT INTO audit_logs (action) VALUES ('probe')")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&app_audit_insert).as_deref(),
        Some("42501"),
        "audit_logs is write-restricted to audit_writer"
    );

    // Denial: cross-owner insert into system-owned shared metadata.
    let cross_owner = sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
    )
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&cross_owner).as_deref(),
        Some("42501"),
        "app role must not INSERT into system-owned shared tables (cross-owner insert)"
    );

    // Denial: immutable shared datasets cannot be mutated either.
    let shared_update = sqlx::query("UPDATE dataset_versions SET status = 'READY'")
        .execute(&app)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&shared_update).as_deref(),
        Some("42501"),
        "shared dataset rows are read-only"
    );

    // Denial: sixth status via app role too.
    let app_sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&app_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&app_sixth).as_deref(),
        Some("23514"),
        "sixth status denied for app too"
    );

    // ------------------------------------------------------------------
    // 7. audit_writer: append-only writer of audit_logs, nothing else.
    // ------------------------------------------------------------------
    let aw = role_pool(super_url, db, "audit_writer").await?;
    sqlx::query("INSERT INTO audit_logs (action, actor_role) VALUES ('qa.probe', 'audit_writer')")
        .execute(&aw)
        .await?;
    let aw_upd = sqlx::query("UPDATE audit_logs SET reason = 'tampered'")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_upd).as_deref(),
        Some("42501"),
        "audit_writer must not UPDATE audit rows"
    );
    let aw_del = sqlx::query("DELETE FROM audit_logs")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_del).as_deref(),
        Some("42501"),
        "audit_writer must not DELETE audit rows"
    );
    let aw_tr = sqlx::query("TRUNCATE TABLE audit_logs")
        .execute(&aw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&aw_tr).as_deref(),
        Some("42501"),
        "audit_writer must not TRUNCATE audit_logs"
    );
    let aw_tenant = sqlx::query(
        "INSERT INTO accounts (owner_user_id, account_type, name) \
                                 VALUES ($1, 'PAPER', 'x')",
    )
    .bind(uid)
    .execute(&aw)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&aw_tenant).as_deref(),
        Some("42501"),
        "audit_writer must not write tenant data (cross-owner insert denied)"
    );

    // ------------------------------------------------------------------
    // 8. Worker role: can claim/advance jobs and attempts, nothing else.
    // ------------------------------------------------------------------
    let wk = role_pool(super_url, db, "worker").await?;
    sqlx::query(
        "UPDATE jobs SET status = 'RUNNING', locked_by = 'worker-probe', locked_at = now() \
         WHERE id = $1 AND status = 'QUEUED'",
    )
    .bind(job_id)
    .execute(&wk)
    .await?;
    sqlx::query(
        "INSERT INTO job_attempts (job_id, attempt_no, outcome, claimed_by) \
         VALUES ($1, 2, 'RUNNING', 'worker-probe')",
    )
    .bind(job_id)
    .execute(&wk)
    .await?;
    let wk_audit = sqlx::query("INSERT INTO audit_logs (action) VALUES ('worker-probe')")
        .execute(&wk)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&wk_audit).as_deref(),
        Some("42501"),
        "worker must not write audit_logs"
    );
    let wk_ddl = sqlx::query("ALTER TABLE jobs DROP COLUMN IF EXISTS priority")
        .execute(&wk)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&wk_ddl).as_deref(),
        Some("42501"),
        "worker must not DDL"
    );
    let wk_tenant = sqlx::query(
        "INSERT INTO accounts (owner_user_id, account_type, name) VALUES ($1, 'PAPER', 'y')",
    )
    .bind(uid)
    .execute(&wk)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&wk_tenant).as_deref(),
        Some("42501"),
        "worker must not write tenant data"
    );

    // ------------------------------------------------------------------
    // 9. Research publication: stable Raw lineage, immutable calendar
    //    history, and a narrowly-scoped publication writer.
    // ------------------------------------------------------------------
    let provenance_columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'data_batches' \
         AND column_name IN ('source_batch_id', 'source_file_name', 'fetch_mode') \
         ORDER BY column_name",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        provenance_columns,
        vec![
            ("fetch_mode".into(), "text".into(), "YES".into()),
            ("source_batch_id".into(), "uuid".into(), "YES".into()),
            ("source_file_name".into(), "text".into(), "YES".into()),
        ],
        "data_batches must expose nullable Raw provenance columns"
    );

    let legacy_batch: Uuid = sqlx::query_scalar(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-02-01', 'REFERENCE', 'data/raw/legacy/1', $1, 1, now()) \
         RETURNING id",
    )
    .bind("d".repeat(64))
    .fetch_one(owner)
    .await?;
    assert!(
        legacy_batch.as_bytes().len() == 16,
        "legacy rows need no provenance"
    );

    let source_batch_id = Uuid::parse_str("00000000-0000-0000-0000-000000000022").unwrap();
    let publication_batch_sql = "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
         VALUES ('KRX', 'KR', '2026-02-02', 'REFERENCE', $1, $2, 1, now(), $3, 'master.csv', $4)";
    sqlx::query(publication_batch_sql)
        .bind("data/raw/published/1")
        .bind("e".repeat(64))
        .bind(source_batch_id)
        .bind("credentialed")
        .execute(owner)
        .await?;
    let duplicate_provenance = sqlx::query(publication_batch_sql)
        .bind("data/raw/published/2")
        .bind("f".repeat(64))
        .bind(source_batch_id)
        .bind("credentialed")
        .execute(owner)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_provenance).as_deref(),
        Some("23505"),
        "published Raw lineage must be unique per provider, market, batch, and file"
    );
    let incomplete_provenance = sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at, source_batch_id) \
         VALUES ('KRX', 'KR', '2026-02-02', 'REFERENCE', 'data/raw/published/incomplete', $1, 1, now(), $2)",
    )
    .bind("0".repeat(64))
    .bind(Uuid::parse_str("00000000-0000-0000-0000-000000000023").unwrap())
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&incomplete_provenance).as_deref(), Some("23514"));
    let invalid_fetch_mode = sqlx::query(publication_batch_sql)
        .bind("data/raw/published/invalid-mode")
        .bind("1".repeat(64))
        .bind(Uuid::parse_str("00000000-0000-0000-0000-000000000024").unwrap())
        .bind("CREDENTIALed")
        .execute(owner)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&invalid_fetch_mode).as_deref(), Some("23514"));
    let publication_constraints: Vec<(String, bool)> = sqlx::query_as(
        "SELECT conname, convalidated FROM pg_constraint WHERE conname IN ( \
         'data_batches_fetch_mode_check', 'data_batches_provenance_all_or_none_check', \
         'trading_calendars_content_sha256_check', 'trading_calendars_provenance_all_or_none_check') \
         ORDER BY conname",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        publication_constraints,
        vec![
            ("data_batches_fetch_mode_check".into(), true),
            ("data_batches_provenance_all_or_none_check".into(), true),
            ("trading_calendars_content_sha256_check".into(), true),
            (
                "trading_calendars_provenance_all_or_none_check".into(),
                true
            ),
        ],
        "publication CHECK constraints must finish validated"
    );
    let source_lineage_index: (bool, bool) = sqlx::query_as(
        "SELECT i.indisunique, i.indpred IS NOT NULL \
         FROM pg_index i WHERE i.indexrelid = 'public.data_batches_source_file_uq'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        source_lineage_index,
        (true, true),
        "source lineage index must be unique and partial"
    );

    let calendar_identity: (String, String, String) = sqlx::query_as(
        "SELECT data_type, is_identity, identity_generation \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'trading_calendar_versions' \
         AND column_name = 'id'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_identity,
        ("bigint".into(), "YES".into(), "ALWAYS".into()),
        "trading_calendar_versions.id must be bigint GENERATED ALWAYS AS IDENTITY"
    );
    let calendar_history_rls: bool = sqlx::query_scalar(
        "SELECT relrowsecurity FROM pg_class WHERE oid = 'public.trading_calendar_versions'::regclass",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_history_rls, "calendar history must enable RLS");

    let calendar_hash = "2".repeat(64);
    let calendar_version_id: i64 = sqlx::query_scalar(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-03', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, $2, now()) \
         RETURNING id",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_version_id > 0,
        "calendar history must use an identity key"
    );
    let duplicate_calendar_version = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-03', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_calendar_version).as_deref(),
        Some("23505")
    );
    let invalid_session_type = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-04', 'OPEN', 'Asia/Seoul', 'KRX', 'bad-session', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_session_type).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid session_type"
    );
    let invalid_timezone = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-05', 'TRADING', 'UTC', 'KRX', 'bad-timezone', $1, $2, now())",
    )
    .bind(source_batch_id)
    .bind(&calendar_hash)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_timezone).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid timezone"
    );
    let invalid_calendar_hash = sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'bad-hash', $1, 'bad', now())",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&invalid_calendar_hash).as_deref(),
        Some("23514"),
        "calendar history must reject only an invalid content hash"
    );
    for reader in ["app", "worker", "admin"] {
        let reader_pool = role_pool(super_url, db, reader).await?;
        let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
            .fetch_one(&reader_pool)
            .await?;
        assert!(
            visible >= 1,
            "{reader} must retain shared calendar history reads"
        );
    }
    for statement in [
        "UPDATE trading_calendar_versions SET source = 'tampered'",
        "DELETE FROM trading_calendar_versions",
    ] {
        let append_only = sqlx::query(statement).execute(owner).await.unwrap_err();
        assert_eq!(
            pg_code(&append_only).as_deref(),
            Some("55000"),
            "{statement}"
        );
    }

    let legacy_calendar: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version) \
         VALUES ('KRX', '2026-02-05', 'CLOSED', 'Asia/Seoul', 'KRX', 'legacy') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        legacy_calendar.as_bytes().len() == 16,
        "legacy calendar rows need no provenance"
    );
    let incomplete_projection = sqlx::query(
        "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version, source_batch_id) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1)",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&incomplete_projection).as_deref(), Some("23514"));
    let invalid_projection_hash = sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-06', 'TRADING', 'Asia/Seoul', 'KRX', 'v1', $1, 'BAD', now())",
    )
    .bind(source_batch_id)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&invalid_projection_hash).as_deref(), Some("23514"));

    let rw = role_pool(super_url, db, "research_writer").await?;
    for (table, expected) in [
        (
            "data_batches",
            (true, true, false, false, false, false, false, false),
        ),
        (
            "trading_calendar_versions",
            (true, true, false, false, false, false, false, false),
        ),
        (
            "trading_calendars",
            (true, true, true, false, false, false, false, false),
        ),
    ] {
        let actual: (bool, bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_table_privilege('research_writer', $1, 'SELECT'), \
                    has_table_privilege('research_writer', $1, 'INSERT'), \
                    has_table_privilege('research_writer', $1, 'UPDATE'), \
                    has_table_privilege('research_writer', $1, 'DELETE'), \
                    has_table_privilege('research_writer', $1, 'TRUNCATE'), \
                    has_table_privilege('research_writer', $1, 'REFERENCES'), \
                    has_table_privilege('research_writer', $1, 'TRIGGER'), \
                    has_table_privilege('research_writer', $1, 'MAINTAIN')",
        )
        .bind(table)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            actual, expected,
            "research_writer ACL must be exact for {table}"
        );
    }
    let writer_policies: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT tablename, policyname, cmd FROM pg_policies \
         WHERE schemaname = 'public' AND 'research_writer' = ANY(roles) \
         ORDER BY tablename, policyname",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        writer_policies,
        vec![
            (
                "data_batches".into(),
                "data_batches_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "data_batches".into(),
                "data_batches_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendar_versions".into(),
                "trading_calendar_versions_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "trading_calendar_versions".into(),
                "trading_calendar_versions_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_insert_research_writer".into(),
                "INSERT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_select_research_writer".into(),
                "SELECT".into()
            ),
            (
                "trading_calendars".into(),
                "trading_calendars_update_research_writer".into(),
                "UPDATE".into()
            ),
        ],
        "research_writer must have only the requested publication RLS policies"
    );
    let sequence_privileges: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'USAGE'), \
                has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'SELECT'), \
                has_sequence_privilege('research_writer', 'public.trading_calendar_versions_id_seq', 'UPDATE')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        sequence_privileges == (false, false, false),
        "research_writer must not have direct identity-sequence privileges"
    );
    let writable_source_batch = Uuid::parse_str("00000000-0000-0000-0000-000000000025").unwrap();
    sqlx::query(publication_batch_sql)
        .bind("data/raw/research-writer/1")
        .bind("3".repeat(64))
        .bind(writable_source_batch)
        .bind("synthetic")
        .execute(&rw)
        .await?;
    let raw_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM data_batches")
        .fetch_one(&rw)
        .await?;
    assert!(
        raw_visible >= 1,
        "research_writer must read published Raw batches"
    );
    sqlx::query(
        "INSERT INTO trading_calendar_versions \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-07', 'TRADING', 'Asia/Seoul', 'KRX', 'writer-v1', $1, $2, now())",
    )
    .bind(writable_source_batch)
    .bind("4".repeat(64))
    .execute(&rw)
    .await?;
    let history_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM trading_calendar_versions")
        .fetch_one(&rw)
        .await?;
    assert!(
        history_visible >= 1,
        "research_writer must read published calendar history"
    );
    let direct_sequence = sqlx::query("SELECT nextval('public.trading_calendar_versions_id_seq')")
        .execute(&rw)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&direct_sequence).as_deref(),
        Some("42501"),
        "research_writer must not advance the calendar identity sequence directly"
    );
    let projection_id: Uuid = sqlx::query_scalar(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', '2026-02-08', 'TRADING', 'Asia/Seoul', 'KRX', 'writer-v1', $1, $2, now()) RETURNING id",
    )
    .bind(writable_source_batch)
    .bind("5".repeat(64))
    .fetch_one(&rw)
    .await?;
    sqlx::query("UPDATE trading_calendars SET source_version = 'writer-v2' WHERE id = $1")
        .bind(projection_id)
        .execute(&rw)
        .await?;
    for statement in [
        "UPDATE data_batches SET kind = 'tampered'",
        "DELETE FROM data_batches",
        "TRUNCATE TABLE data_batches",
        "DELETE FROM trading_calendars",
        "DELETE FROM trading_calendar_versions",
        "UPDATE trading_calendar_versions SET source = 'tampered'",
        "SELECT * FROM orders",
        "SELECT * FROM jobs",
        "SELECT * FROM audit_logs",
        "CREATE TABLE research_writer_hack (id integer)",
    ] {
        let denied = sqlx::query(statement).execute(&rw).await.unwrap_err();
        assert_eq!(pg_code(&denied).as_deref(), Some("42501"), "{statement}");
    }

    // ------------------------------------------------------------------
    // 10. sha256-hash columns enforce `^[0-9a-f]{64}$` (immutable manifests).
    // ------------------------------------------------------------------
    let bad_hash = sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-01-05', 'EOD', 'data/raw/qa/1', 'not-a-hash', 1, now())",
    )
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_hash).as_deref(),
        Some("23514"),
        "sha256 columns must reject non-hex hashes"
    );
    let good_hash = "a".repeat(64);
    sqlx::query(
        "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, \
         content_sha256, bytes_size, retrieved_at) \
         VALUES ('KRX', 'KR', '2026-01-05', 'EOD', 'data/raw/qa/2', $1, 1, now())",
    )
    .bind(&good_hash)
    .execute(owner)
    .await?;

    // Large curves/orders/fills live in Parquet with DB manifests.
    let bad_artifact_hash = sqlx::query(
        "INSERT INTO result_artifacts (backtest_run_id, owner_user_id, artifact_type, \
         parquet_path, row_count, sha256, size_bytes) \
         VALUES ('00000000-0000-0000-0000-000000000001', $1, 'EQUITY_CURVE', \
         'data/artifacts/qa/1.parquet', 0, 'zz', 0)",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_artifact_hash).as_deref(),
        Some("23514"),
        "result_artifacts.sha256 must be 64 hex chars"
    );

    // ------------------------------------------------------------------
    // 10. web_sessions: opaque-hash identity, unique.
    // ------------------------------------------------------------------
    let sess_hash = "b".repeat(64);
    let csrf_hash = "c".repeat(64);
    sqlx::query(
        "INSERT INTO web_sessions (user_id, session_hash, csrf_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(uid)
    .bind(&sess_hash)
    .bind(&csrf_hash)
    .execute(&owner_actor)
    .await?;
    let dup_session = sqlx::query(
        "INSERT INTO web_sessions (user_id, session_hash, csrf_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(uid)
    .bind(&sess_hash)
    .bind(&csrf_hash)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&dup_session).as_deref(),
        Some("23505"),
        "web_sessions.session_hash must be unique"
    );

    // ------------------------------------------------------------------
    // 11. data_entitlements lifecycle CHECK (PENDING|ACTIVE|EXPIRED|REVOKED).
    // ------------------------------------------------------------------
    sqlx::query(
        "INSERT INTO data_entitlements (contract_document_sha256, contract_reference, status, \
         covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES ($1, 'ref/qa/1', 'PENDING', '[\"krx-eod\"]'::jsonb, '[\"backtest\"]'::jsonb, \
         '2026-01-01', '2026-12-31', $2)",
    )
    .bind(&good_hash)
    .bind(uid)
    .execute(owner)
    .await?;
    let bad_entitlement = sqlx::query(
        "INSERT INTO data_entitlements (contract_document_sha256, contract_reference, status, \
         covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
         VALUES ($1, 'ref/qa/2', 'BOGUS', '[]'::jsonb, '[]'::jsonb, '2026-01-01', '2026-12-31', $2)",
    )
    .bind(&good_hash)
    .bind(uid)
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&bad_entitlement).as_deref(),
        Some("23514"),
        "data_entitlements.status must be CHECK-enforced"
    );

    // ------------------------------------------------------------------
    // 12. Risk gateway (0018): one immutable decision per intent.
    //
    // Every assertion here corresponds to a claim the migration's comments
    // make. A gate decision that could be edited, duplicated, or written
    // half-populated would not be evidence of why an order was allowed, which
    // is the only reason the row exists.
    // ------------------------------------------------------------------
    sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('contract-v1', 3000, 1000000, 5000000, 500000, 300)",
    )
    .execute(owner)
    .await?;

    // A limit set that would deny every order is a misconfiguration, refused
    // by CHECK exactly as the crate's constructor refuses it.
    let zero_limit = sqlx::query(
        "INSERT INTO risk_limits (version, max_symbol_weight_bp, max_order_value, \
         max_daily_order_value, max_daily_loss, max_data_age_secs) \
         VALUES ('contract-bad', 3000, 0, 5000000, 500000, 300)",
    )
    .execute(owner)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&zero_limit).as_deref(),
        Some("23514"),
        "a zero max_order_value must be CHECK-refused"
    );

    let gate_insert = "INSERT INTO risk_events (owner_user_id, event_type, severity, \
         intent_ref, correlation_id, limits_version, decision, denied_by_check, reason_code, \
         evaluated_at) VALUES ($1, 'LIVE_ORDER_GATE', $2, $3, 'corr-1', 'contract-v1', $4, $5, \
         $6, now())";

    sqlx::query(gate_insert)
        .bind(uid)
        .bind("INFO")
        .bind("intent-contract-1")
        .bind("APPROVED")
        .bind(Option::<String>::None)
        .bind("APPROVED")
        .execute(&owner_actor)
        .await?;

    // One decision per intent, enforced by the partial unique index.
    let duplicate = sqlx::query(gate_insert)
        .bind(uid)
        .bind("WARNING")
        .bind("intent-contract-1")
        .bind("DENIED")
        .bind(Some("KILL_SWITCH"))
        .bind("LIVE_KILL_SWITCH_ENGAGED")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate).as_deref(),
        Some("23505"),
        "an intent may carry exactly one gate decision"
    );

    // A half-populated decision is the shape a partial write would take.
    let incomplete = sqlx::query(
        "INSERT INTO risk_events (owner_user_id, event_type, intent_ref, decision) \
         VALUES ($1, 'LIVE_ORDER_GATE', 'intent-contract-2', 'APPROVED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&incomplete).as_deref(),
        Some("23514"),
        "a gate decision missing its limits version or correlation id must be refused"
    );

    // An approval that names a denying check is self-contradictory.
    let contradiction = sqlx::query(gate_insert)
        .bind(uid)
        .bind("INFO")
        .bind("intent-contract-3")
        .bind("APPROVED")
        .bind(Some("KILL_SWITCH"))
        .bind("APPROVED")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&contradiction).as_deref(),
        Some("23514"),
        "an APPROVED decision must not name a denying check"
    );

    // The constraint applies to gate decisions only: other risk events keep
    // 0007's shape and need none of these columns.
    sqlx::query("INSERT INTO risk_events (owner_user_id, event_type) VALUES ($1, 'RATE_LIMIT')")
        .bind(uid)
        .execute(&owner_actor)
        .await?;

    // Append-only, even for the migration owner: the trigger refuses both.
    let updated = sqlx::query("UPDATE risk_events SET decision = 'DENIED' WHERE intent_ref = $1")
        .bind("intent-contract-1")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&updated).as_deref(),
        Some("42501"),
        "a recorded risk decision must not be editable"
    );
    let deleted = sqlx::query("DELETE FROM risk_events WHERE intent_ref = $1")
        .bind("intent-contract-1")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&deleted).as_deref(),
        Some("42501"),
        "a recorded risk decision must not be deletable"
    );

    // And `app` -- the role the API actually runs as -- holds no grant to try.
    for statement in [
        "UPDATE risk_events SET decision = 'DENIED'",
        "DELETE FROM risk_events",
    ] {
        let denied = sqlx::query(statement).execute(&app).await.unwrap_err();
        assert_eq!(
            pg_code(&denied).as_deref(),
            Some("42501"),
            "app must hold no mutation grant on risk_events: {statement}"
        );
    }

    // ------------------------------------------------------------------
    // 13. Order intents (0019): the constraints the migration claims.
    //
    // The api-server suite proves the repository behaves; these prove the
    // SCHEMA refuses the shapes the repository is trusted not to write, so a
    // future writer -- or a psql session -- cannot create them either.
    // ------------------------------------------------------------------
    // 0019's FK requires the instrument to exist; this database seeds none.
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency, name, asset_class, status)          VALUES ('069500.KRX', '069500', 'KRX', 'KRW', 'KODEX 200', 'ETF', 'ACTIVE')          ON CONFLICT (id) DO NOTHING",
    )
    .execute(owner)
    .await?;

    let account_id: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO accounts (owner_user_id, account_type, name, currency)          VALUES ($1, 'LIVE', 'contract-live', 'KRW') RETURNING id",
    )
    .bind(uid)
    .fetch_one(&owner_actor)
    .await?;

    let intent_insert = "INSERT INTO order_intents          (intent_ref, owner_user_id, account_id, instrument_id, side, quantity, price,           correlation_id, state, broker_order_no, cumulative_filled)          VALUES ($1, $2, $3, '069500.KRX', $4, $5::numeric, 7250, 'corr', $6, $7, $8::numeric)";

    // A well-formed intent.
    sqlx::query(intent_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("INTENT_CREATED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await?;

    // A state that names a broker order must HAVE one: an ACCEPTED row with
    // no order number is a row nobody can reconcile against the broker.
    let unbound = sqlx::query(intent_insert)
        .bind("oi-contract-2")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("ACCEPTED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&unbound).as_deref(),
        Some("23514"),
        "ACCEPTED without a broker order number must be refused"
    );

    // A fill beyond the order quantity.
    let overfilled = sqlx::query(intent_insert)
        .bind("oi-contract-3")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("PARTIALLY_FILLED")
        .bind(Some("B-1"))
        .bind("11")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&overfilled).as_deref(),
        Some("23514"),
        "cumulative_filled above the order quantity must be refused"
    );

    // An unknown state string. The set is closed so a typo cannot create a
    // state the machine has never heard of.
    let bogus_state = sqlx::query(intent_insert)
        .bind("oi-contract-4")
        .bind(uid)
        .bind(account_id)
        .bind("BUY")
        .bind("10")
        .bind("ALMOST_FILLED")
        .bind(Option::<String>::None)
        .bind("0")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&bogus_state).as_deref(), Some("23514"));

    // One broker order belongs to one intent, in both directions.
    for (r, no) in [("oi-contract-5", "B-UNIQ"), ("oi-contract-6", "B-UNIQ")] {
        let result = sqlx::query(intent_insert)
            .bind(r)
            .bind(uid)
            .bind(account_id)
            .bind("BUY")
            .bind("10")
            .bind("ACCEPTED")
            .bind(Some(no))
            .bind("0")
            .execute(&owner_actor)
            .await;
        if r == "oi-contract-6" {
            assert_eq!(
                pg_code(&result.unwrap_err()).as_deref(),
                Some("23505"),
                "two intents must not claim one broker order"
            );
        } else {
            result?;
        }
    }

    // The event log: gapless per intent, and append-only.
    let event_insert = "INSERT INTO order_intent_events          (intent_ref, owner_user_id, seq, event_type, resulting_state)          VALUES ($1, $2, $3, $4, $5)";
    sqlx::query(event_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(1_i32)
        .bind("RISK_APPROVED")
        .bind("RISK_APPROVED")
        .execute(&owner_actor)
        .await?;
    let duplicate_seq = sqlx::query(event_insert)
        .bind("oi-contract-1")
        .bind(uid)
        .bind(1_i32)
        .bind("SUBMISSION_STARTED")
        .bind("SUBMITTING")
        .execute(&owner_actor)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&duplicate_seq).as_deref(),
        Some("23505"),
        "two events must not share a sequence number within one intent"
    );

    // History does not change, not even for the migration owner.
    for statement in [
        "UPDATE order_intent_events SET resulting_state = 'FILLED'",
        "DELETE FROM order_intent_events",
    ] {
        let err = sqlx::query(sqlx::AssertSqlSafe(statement.to_string()))
            .execute(&owner_actor)
            .await
            .unwrap_err();
        assert_eq!(pg_code(&err).as_deref(), Some("42501"), "{statement}");
    }

    // But the intent row ITSELF is mutable: its state legitimately moves, and
    // fencing it would have been copying the 0018 pattern without the reason.
    sqlx::query("UPDATE order_intents SET state = 'RISK_APPROVED' WHERE intent_ref = $1")
        .bind("oi-contract-1")
        .execute(&owner_actor)
        .await?;

    drop(app);
    drop(aw);
    drop(wk);
    Ok(())
}

/// Revert (undo all migrations) then run again in a disposable DB: the schema
/// must be fully removed by the down scripts and fully restored by re-run.
///
/// Un-gated since Todo 3's live gate passed: runs with `cargo test -p
/// migration-contract` against `DATABASE_URL` (disposable scratch DB).
#[tokio::test]
async fn revert_and_rerun_in_disposable_db() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = revert_and_rerun_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("revert-and-rerun FAILED: {e}");
    }
}

/// The online-migration sequence has observable safe boundaries: 0022 adds
/// NOT VALID checks under brief metadata locks, 0023 validates them after that
/// transaction commits, and 0024 builds the populated-table index concurrently.
#[tokio::test]
async fn research_publication_migration_boundaries() {
    let super_url = match require_db_url() {
        Ok(u) => u,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(v) => v,
        Err(e) => panic!("setup failed: {e}"),
    };
    let result = research_publication_boundaries_body(&owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(e) = result {
        panic!("research-publication migration boundaries FAILED: {e}");
    }
}

async fn research_publication_boundaries_body(owner: &PgPool) -> Result<(), Box<dyn Error>> {
    const PUBLICATION_CHECKS: [&str; 4] = [
        "data_batches_fetch_mode_check",
        "data_batches_provenance_all_or_none_check",
        "trading_calendars_content_sha256_check",
        "trading_calendars_provenance_all_or_none_check",
    ];

    let validation_state = |expected: bool| async move {
        let checks: Vec<(String, bool)> = sqlx::query_as(
            "SELECT conname, convalidated FROM pg_constraint WHERE conname = ANY($1) ORDER BY conname",
        )
        .bind(PUBLICATION_CHECKS.as_slice())
        .fetch_all(owner)
        .await?;
        let expected_checks = PUBLICATION_CHECKS
            .iter()
            .map(|name| (name.to_string(), expected))
            .collect::<Vec<_>>();
        Ok::<_, sqlx::Error>((checks, expected_checks))
    };

    MIGRATOR.run_to(22, owner).await?;
    assert_eq!(applied_count(owner).await?, 22, "0022 must apply alone");
    let (checks_after_0022, expected_unvalidated) = validation_state(false).await?;
    assert_eq!(
        checks_after_0022, expected_unvalidated,
        "0022 must leave populated-table checks present but NOT VALID"
    );
    let source_batch_id = Uuid::parse_str("00000000-0000-0000-0000-000000000026").unwrap();
    let invalid_writes = [
        (
            "INSERT INTO data_batches (provider, market, batch_date, kind, storage_path, content_sha256, bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode) \
             VALUES ('KRX', 'KR', '2026-03-01', 'REFERENCE', 'data/raw/boundary/fetch', $1, 1, now(), $2, 'source.csv', 'INVALID')",
            true,
        ),
        (
            "INSERT INTO trading_calendars (exchange, session_date, session_type, timezone, source, source_version, source_batch_id, content_sha256, retrieved_at) \
             VALUES ('KRX', '2026-03-01', 'TRADING', 'Asia/Seoul', 'KRX', 'boundary', $1, 'invalid', now())",
            false,
        ),
    ];
    for (statement, has_hash_parameter) in invalid_writes {
        let mut query = sqlx::query(statement);
        if has_hash_parameter {
            query = query.bind("a".repeat(64));
        }
        let invalid = query
            .bind(source_batch_id)
            .execute(owner)
            .await
            .unwrap_err();
        assert_eq!(
            pg_code(&invalid).as_deref(),
            Some("23514"),
            "NOT VALID checks must still reject invalid new publication writes"
        );
    }

    MIGRATOR.run_to(23, owner).await?;
    assert_eq!(applied_count(owner).await?, 23, "0023 must validate checks");
    let (checks_after_0023, expected_validated) = validation_state(true).await?;
    assert_eq!(
        checks_after_0023, expected_validated,
        "0023 must finish publication checks validated"
    );

    MIGRATOR.run_to(24, owner).await?;
    assert_eq!(
        applied_count(owner).await?,
        24,
        "0024 must add the concurrent source index"
    );
    let source_index: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(source_index.as_deref(), Some("data_batches_source_file_uq"));
    let calendar_lookup_before_0025: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_before_0025.is_none());

    let expected = up_migration_count() as i64;
    MIGRATOR.run(owner).await?;
    assert_eq!(applied_count(owner).await?, expected);
    let calendar_lookup: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    MIGRATOR.undo(owner, 24).await?;
    assert_eq!(applied_count(owner).await?, 24, "0025 down must run first");
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_gone.is_none());
    let source_index_retained: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        source_index_retained.as_deref(),
        Some("data_batches_source_file_uq")
    );

    MIGRATOR.undo(owner, 23).await?;
    assert_eq!(applied_count(owner).await?, 23, "0024 down must run second");
    let source_index_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        source_index_gone.is_none(),
        "0024 down must remove the index"
    );
    let (checks_after_0024_down, expected_still_validated) = validation_state(true).await?;
    assert_eq!(checks_after_0024_down, expected_still_validated);

    MIGRATOR.undo(owner, 22).await?;
    assert_eq!(applied_count(owner).await?, 22, "0023 down must run third");
    let (checks_after_0023_down, expected_restored_unvalidated) = validation_state(false).await?;
    assert_eq!(
        checks_after_0023_down, expected_restored_unvalidated,
        "0023 down must restore 0022's NOT VALID boundary"
    );

    MIGRATOR.undo(owner, 21).await?;
    assert_eq!(applied_count(owner).await?, 21, "0022 down must run last");
    let history_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.trading_calendar_versions')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        history_gone.is_none(),
        "0022 down must remove calendar history"
    );

    MIGRATOR.run(owner).await?;
    assert_eq!(
        applied_count(owner).await?,
        expected,
        "full reapply must restore all publication migration boundaries"
    );
    Ok(())
}

async fn revert_and_rerun_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let expected = up_migration_count();
    MIGRATOR.run(owner).await?;
    let applied = applied_count(owner).await? as usize;
    assert_eq!(
        applied, expected,
        "fresh DB must apply all {expected} migrations"
    );

    // Revert the 0037..0026 recommendation family, then 0025, 0024, and 0023
    // before 0022 while all earlier tables remain.
    // This proves each down migration restores its own boundary rather than
    // relying on 0003.down to hide omitted objects in a full teardown.
    MIGRATOR.undo(owner, 25).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        25,
        "undo to 0025 must revert the complete 0037..0026 family"
    );
    let scheduler_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        scheduler_gone.is_none(),
        "0026 down must remove its function"
    );
    let calendar_lookup_retained: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup_retained.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    MIGRATOR.undo(owner, 24).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        24,
        "undo to 0024 must revert only 0025"
    );
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_lookup_gone.is_none(),
        "0025 down must remove the concurrent calendar source-version index"
    );
    let source_index_retained: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        source_index_retained.as_deref(),
        Some("data_batches_source_file_uq")
    );

    MIGRATOR.undo(owner, 23).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        23,
        "undo to 0023 must revert only 0024"
    );
    let source_index_gone: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.data_batches_source_file_uq')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        source_index_gone.is_none(),
        "0024 down must remove the concurrent source-lineage index"
    );
    MIGRATOR.undo(owner, 22).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        22,
        "undo to 0022 must revert only 0023"
    );
    sqlx::query(
        "CREATE UNIQUE INDEX data_batches_source_file_uq \
         ON data_batches (provider, market, source_batch_id, source_file_name) \
         WHERE source_batch_id IS NOT NULL",
    )
    .execute(owner)
    .await?;
    MIGRATOR.undo(owner, 21).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        21,
        "undo to 0021 must revert only 0022"
    );
    for object in [
        "public.trading_calendar_versions",
        "public.data_batches_source_file_uq",
        "public.trading_calendar_versions_source_lookup_idx",
    ] {
        let gone: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(object)
            .fetch_one(owner)
            .await?;
        assert!(gone.is_none(), "0022 down must remove {object}");
    }
    let remaining_0022_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = 'public' \
         AND ((table_name = 'data_batches' AND column_name IN ('source_batch_id', 'source_file_name', 'fetch_mode')) \
           OR (table_name = 'trading_calendars' AND column_name IN ('source_batch_id', 'content_sha256', 'retrieved_at')))",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_columns, 0,
        "0022 down must remove added columns"
    );
    let remaining_0022_constraints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE conname IN ( \
         'data_batches_fetch_mode_check', 'data_batches_provenance_all_or_none_check', \
         'trading_calendars_content_sha256_check', 'trading_calendars_provenance_all_or_none_check')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_constraints, 0,
        "0022 down must remove added constraints"
    );
    let function_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure('public.trading_calendar_versions_reject_mutation()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        function_gone.is_none(),
        "0022 down must remove its trigger function"
    );
    let remaining_0022_policies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_policies WHERE policyname IN ( \
         'data_batches_select_research_writer', 'data_batches_insert_research_writer', \
         'trading_calendars_select_research_writer', 'trading_calendars_insert_research_writer', \
         'trading_calendars_update_research_writer', 'trading_calendar_versions_select_readers', \
         'trading_calendar_versions_select_research_writer', 'trading_calendar_versions_insert_research_writer')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        remaining_0022_policies, 0,
        "0022 down must remove its RLS policies"
    );
    for (table, privilege) in [("data_batches", "INSERT"), ("trading_calendars", "UPDATE")] {
        let retained: bool =
            sqlx::query_scalar("SELECT has_table_privilege('research_writer', $1, $2)")
                .bind(table)
                .bind(privilege)
                .fetch_one(owner)
                .await?;
        assert!(
            !retained,
            "0022 down must revoke research_writer {privilege} on {table}"
        );
    }
    let research_writer_survives: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'research_writer')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        research_writer_survives,
        "0022 down must retain the externally created research_writer role"
    );

    MIGRATOR.run(owner).await?;
    assert_eq!(
        applied_count(owner).await? as usize,
        expected,
        "re-applying after 0022-only undo must restore 0022"
    );
    let calendar_history_restored: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.trading_calendar_versions')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        calendar_history_restored.as_deref(),
        Some("trading_calendar_versions"),
        "0022 up must restore calendar history after its standalone down"
    );
    let calendar_lookup_restored: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup_restored.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    // Undo every migration. sqlx 0.9's `undo` reverts migrations whose version
    // is > target; target 0 therefore reverts everything (the pre-fix code
    // passed `expected`, which would have reverted nothing).
    MIGRATOR.undo(owner, 0).await?;
    let remaining = applied_count(owner).await?;
    assert_eq!(remaining, 0, "undo must revert all {expected} migrations");

    // Schema objects are gone.
    let jobs_gone: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.jobs')::text")
            .fetch_one(owner)
            .await?;
    assert!(
        jobs_gone.is_none(),
        "after revert, public.jobs must not exist"
    );
    let calendar_history_gone: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('public.trading_calendar_versions')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        calendar_history_gone.is_none(),
        "after revert, public.trading_calendar_versions must not exist"
    );
    let calendar_lookup_gone: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(calendar_lookup_gone.is_none());

    // Run again from scratch.
    MIGRATOR.run(owner).await?;
    let applied2 = applied_count(owner).await? as usize;
    assert_eq!(
        applied2, expected,
        "re-run after revert must re-apply everything"
    );
    let audit_back: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('public.audit_logs')::text")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        audit_back.as_deref(),
        Some("audit_logs"),
        "audit_logs must exist after re-run"
    );
    let calendar_history_back: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('public.trading_calendar_versions')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_history_back.as_deref(),
        Some("trading_calendar_versions"),
        "trading_calendar_versions must exist after re-run"
    );
    let calendar_lookup_back: Option<String> = sqlx::query_scalar(
        "SELECT to_regclass('public.trading_calendar_versions_source_lookup_idx')::text",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        calendar_lookup_back.as_deref(),
        Some("trading_calendar_versions_source_lookup_idx")
    );

    // Post-revert DB still enforces the five-state contract (deterministic).
    let uid: Uuid = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test','revert-owner','revert@example.test') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    // The RLS policy check precedes constraint evaluation (PG18), so the
    // ORPHANED probe runs under an actor context to reach the CHECK.
    let owner_actor = actor_pool(super_url, db, "migration_owner", &uid.to_string()).await?;
    let sixth = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, status) VALUES ($1, 'backtest', 'ORPHANED')",
    )
    .bind(uid)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&sixth).as_deref(),
        Some("23514"),
        "sixth status still rejected after revert+run"
    );

    let _ = super_url;
    Ok(())
}

#[tokio::test]
async fn recommendation_pipeline_migration_contract() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = recommendation_pipeline_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("recommendation pipeline migration contract FAILED: {error}");
    }
}

#[tokio::test]
async fn paper_rebalance_preview_migration_contract() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = paper_rebalance_preview_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("Paper rebalance preview migration contract FAILED: {error}");
    }
}

async fn paper_rebalance_preview_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run(owner).await?;

    let owner_constraints_validated: bool = sqlx::query_scalar(
        "SELECT count(*)=2 AND bool_and(convalidated) \
           FROM pg_constraint \
          WHERE conname IN ('cash_ledger_account_owner_fkey', \
                            'positions_account_owner_fkey')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        owner_constraints_validated,
        "Paper account-owner foreign keys must be fully validated"
    );
    let temporary_policy_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_policies \
          WHERE policyname LIKE 'paper_preview_migration_validate_%'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        temporary_policy_count, 0,
        "migration-only cross-tenant policies must not survive commit"
    );

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind("paper-preview-contract")
    .bind("paper-preview-contract@example.test")
    .fetch_one(owner)
    .await?;
    let owner_actor = actor_pool(super_url, db, "migration_owner", &user_id.to_string()).await?;
    let app = actor_pool(super_url, db, "app", &user_id.to_string()).await?;
    let worker = role_pool(super_url, db, "worker").await?;

    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('paper_preview_contract', 'Paper Preview Contract', 'Paper')",
    )
    .execute(owner)
    .await?;
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
    )
    .execute(owner)
    .await?;
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'paper_preview_contract', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts \
         (owner_user_id, account_type, name, status, initial_cash) \
         VALUES ($1, 'PAPER', 'paper-preview', 'ACTIVE', 1000000) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version) \
         VALUES ($1, $2, $3, 'paper_preview_contract', '1.0.0')",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .execute(&owner_actor)
    .await?;
    let dataset_version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('krx_eod_bars', 'preview-v1', 'READY', repeat('a', 64), \
                 'curated/preview-v1') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, \
          covered_uses, effective_from, effective_until, managed_by) \
         VALUES (repeat('e', 64), 'vault://qa/paper-preview', 'ACTIVE', \
                 '[\"krx_eod_bars\"]', '[\"recommendation\"]', \
                 DATE '2020-01-01', DATE '2030-12-31', $1)",
    )
    .bind(user_id)
    .execute(owner)
    .await?;
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, \
          source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', DATE '2026-08-13', 'TRADING', 'Asia/Seoul', 'KRX', \
                 'preview-v1', $1, repeat('c', 64), now())",
    )
    .bind(Uuid::parse_str("00000000-0000-0000-0000-000000000038")?)
    .execute(owner)
    .await?;

    let recommendation_job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs \
         (owner_user_id, job_type, status, idempotency_key, payload_json) \
         VALUES ($1, 'recommendation', 'SUCCEEDED', $2, \
                 jsonb_build_object('dataset', jsonb_build_object( \
                   'id', $3::uuid, 'dataset_id', 'krx_eod_bars', \
                   'version', 'preview-v1', 'curated_version', 7, \
                   'manifest_sha256', repeat('a',64)))) RETURNING id",
    )
    .bind(user_id)
    .bind("preview-source-contract")
    .bind(dataset_version_id)
    .fetch_one(&owner_actor)
    .await?;
    let recommendation_run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, \
          dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, DATE '2026-08-11', 'SUCCEEDED', $3, 'MANUAL', \
                 $4, repeat('a', 64)) RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(recommendation_job_id)
    .bind(dataset_version_id)
    .fetch_one(&owner_actor)
    .await?;
    let portfolio_id: Uuid = sqlx::query_scalar(
        "INSERT INTO target_portfolios \
         (owner_user_id, recommendation_run_id, as_of, weights_json) \
         VALUES ($1, $2, DATE '2026-08-11', \
                 '{\"069500.KRX\":\"1.000000\"}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .bind(recommendation_run_id)
    .fetch_one(&owner_actor)
    .await?;

    let submission: (String, Option<Uuid>) = sqlx::query_as(
        "SELECT outcome, target_portfolio_id \
         FROM lock_paper_rebalance_preview_submission($1, $2, $3, DATE '2026-08-12')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(recommendation_run_id)
    .fetch_one(&app)
    .await?;
    assert_eq!(submission, ("READY".into(), Some(portfolio_id)));
    let worker_submission = sqlx::query(
        "SELECT outcome \
         FROM lock_paper_rebalance_preview_submission($1, $2, $3, DATE '2026-08-12')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(recommendation_run_id)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&worker_submission).as_deref(), Some("42501"));

    let mut expected_version = 0_i64;
    let read_version = || {
        sqlx::query_scalar::<_, i64>("SELECT paper_state_version FROM accounts WHERE id = $1")
            .bind(account_id)
            .fetch_one(&owner_actor)
    };
    assert_eq!(read_version().await?, expected_version);

    let attacker_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', $1, $2) RETURNING id",
    )
    .bind("paper-preview-attacker")
    .bind("paper-preview-attacker@example.test")
    .fetch_one(owner)
    .await?;
    let attacker = actor_pool(super_url, db, "app", &attacker_id.to_string()).await?;
    let forged_cash = sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance) \
         VALUES ($1, $2, 900, 'FORGED', 0, 0)",
    )
    .bind(account_id)
    .bind(attacker_id)
    .execute(&attacker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&forged_cash).as_deref(), Some("23503"));
    let forged_position = sqlx::query(
        "INSERT INTO positions (account_id, owner_user_id, instrument_id, quantity) \
         VALUES ($1, $2, '069500.KRX', 1)",
    )
    .bind(account_id)
    .bind(attacker_id)
    .execute(&attacker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&forged_position).as_deref(), Some("23503"));
    assert_eq!(read_version().await?, expected_version);

    let cascade_account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts \
         (owner_user_id, account_type, name, status, initial_cash) \
         VALUES ($1, 'PAPER', 'paper-preview-cascade', 'ACTIVE', 1) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance) \
         VALUES ($1, $2, 1, 'DEPOSIT', 1, 1)",
    )
    .bind(cascade_account_id)
    .bind(user_id)
    .execute(&owner_actor)
    .await?;
    sqlx::query(
        "INSERT INTO positions (account_id, owner_user_id, instrument_id, quantity) \
         VALUES ($1, $2, '069500.KRX', 1)",
    )
    .bind(cascade_account_id)
    .bind(user_id)
    .execute(&owner_actor)
    .await?;
    sqlx::query("DELETE FROM accounts WHERE id=$1")
        .bind(cascade_account_id)
        .execute(&owner_actor)
        .await?;

    let cash_id: Uuid = sqlx::query_scalar(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance) \
         VALUES ($1, $2, 1, 'DEPOSIT', 1000000, 1000000) RETURNING id",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    sqlx::query("UPDATE cash_ledger SET ts = ts + interval '1 second' WHERE id = $1")
        .bind(cash_id)
        .execute(&owner_actor)
        .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    sqlx::query("DELETE FROM cash_ledger WHERE id = $1")
        .bind(cash_id)
        .execute(&owner_actor)
        .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    let position_id: Uuid = sqlx::query_scalar(
        "INSERT INTO positions (account_id, owner_user_id, instrument_id, quantity) \
         VALUES ($1, $2, '069500.KRX', 1) RETURNING id",
    )
    .bind(account_id)
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    sqlx::query("UPDATE positions SET quantity = 2 WHERE id = $1")
        .bind(position_id)
        .execute(&owner_actor)
        .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    sqlx::query("DELETE FROM positions WHERE id = $1")
        .bind(position_id)
        .execute(&owner_actor)
        .await?;
    expected_version += 1;
    assert_eq!(read_version().await?, expected_version);
    sqlx::query(
        "INSERT INTO cash_ledger \
         (account_id, owner_user_id, seq, event_type, amount, balance) \
         VALUES ($1, $2, 2, 'DEPOSIT', 1000000, 1000000)",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&owner_actor)
    .await?;
    expected_version += 1;

    let preview_job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, status, idempotency_key, payload_json) \
         VALUES ($1, 'paper_rebalance_preview', 'RUNNING', $2, '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .bind("paper-preview:contract")
    .fetch_one(&app)
    .await?;
    let preview_id: Uuid = sqlx::query_scalar(
        "INSERT INTO paper_rebalance_previews \
         (owner_user_id, account_id, recommendation_run_id, target_portfolio_id, \
          strategy_config_id, job_id, price_date, dataset_version_id, \
          dataset_manifest_sha256, target_portfolio_sha256) \
         VALUES ($1, $2, $3, $4, $5, $6, DATE '2026-08-11', $7, \
                 repeat('a', 64), repeat('b', 64)) RETURNING id",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(recommendation_run_id)
    .bind(portfolio_id)
    .bind(config_id)
    .bind(preview_job_id)
    .bind(dataset_version_id)
    .fetch_one(&app)
    .await?;

    let app_publish = sqlx::query(
        "SELECT publish_paper_rebalance_preview( \
         $1, $2, $3, repeat('d',64), 'KRX_ETF_DEFAULT', 1, DATE '2026-08-13', \
         repeat('f',64), '{\"069500.KRX\":\"1.000000\"}'::jsonb, '{}'::jsonb)",
    )
    .bind(preview_id)
    .bind(preview_job_id)
    .bind(expected_version)
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&app_publish).as_deref(), Some("42501"));
    let worker_apply = sqlx::query(
        "SELECT * FROM apply_paper_rebalance_preview($1, $2, repeat('f',64), DATE '2026-08-12')",
    )
    .bind(user_id)
    .bind(preview_id)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&worker_apply).as_deref(), Some("42501"));
    let forged_origin = sqlx::query(
        "INSERT INTO pending_targets \
         (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, \
          targets_json, dataset_version, dataset_version_id, dataset_manifest_sha256, \
          source_kind, recommendation_run_id) \
         VALUES ($1, $2, $3, DATE '2026-08-11', DATE '2026-08-13', '[]'::jsonb, \
                 'preview-v1', $4, repeat('a',64), 'MANUAL_RECOMMENDATION', $5)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind(recommendation_run_id)
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&forged_origin).as_deref(), Some("42501"));

    let snapshot_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM snapshot_paper_rebalance_preview( \
         $1, $2, DATE '2026-08-12')",
    )
    .bind(preview_id)
    .bind(preview_job_id)
    .fetch_one(&worker)
    .await?;
    assert_eq!(
        snapshot_count, 1,
        "worker must resolve one exact preview snapshot"
    );
    let published: bool = sqlx::query_scalar(
        "SELECT publish_paper_rebalance_preview( \
         $1, $2, $3, repeat('d',64), 'KRX_ETF_DEFAULT', 1, DATE '2026-08-13', \
         repeat('f',64), '{\"069500.KRX\":\"1.000000\"}'::jsonb, \
         '{\"schema_version\":1}'::jsonb)",
    )
    .bind(preview_id)
    .bind(preview_job_id)
    .bind(expected_version)
    .fetch_one(&worker)
    .await?;
    assert!(published);
    let applied: (String, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT outcome, pending_target_id, source_kind \
         FROM apply_paper_rebalance_preview($1, $2, repeat('f',64), DATE '2026-08-12')",
    )
    .bind(user_id)
    .bind(preview_id)
    .fetch_one(&app)
    .await?;
    assert_eq!(applied.0, "APPLIED");
    assert!(applied.1.is_some());
    assert_eq!(applied.2.as_deref(), Some("MANUAL_RECOMMENDATION"));
    let manual_lineage: (String, Uuid) = sqlx::query_as(
        "SELECT source_kind, recommendation_run_id \
         FROM pending_targets WHERE id = $1",
    )
    .bind(applied.1.unwrap())
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(
        manual_lineage,
        ("MANUAL_RECOMMENDATION".into(), recommendation_run_id)
    );

    Ok(())
}

async fn recommendation_pipeline_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run_to(25, owner).await?;
    assert_eq!(applied_count(owner).await?, 25);

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'recommendation-owner', 'recommendation@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('recommendation_contract', 'Recommendation Contract', 'Paper')",
    )
    .execute(owner)
    .await?;
    let owner_actor = actor_pool(super_url, db, "migration_owner", &user_id.to_string()).await?;
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'recommendation_contract', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let dataset_version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('kr-etf-core', '2026-08-11', 'READY', $1, 'curated/kr-etf-core/2026-08-11') \
         RETURNING id",
    )
    .bind("a".repeat(64))
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('069500.KRX', '069500', 'KRX', 'KRW')",
    )
    .execute(owner)
    .await?;
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, status) \
         VALUES ($1, 'PAPER', 'recommendation-paper', 'ACTIVE') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let binding_id: Uuid = sqlx::query_scalar(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version) \
         VALUES ($1, $2, $3, 'recommendation_contract', '1.0.0') RETURNING id",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .fetch_one(&owner_actor)
    .await?;
    let legacy_run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs (owner_user_id, strategy_config_id, as_of) \
         VALUES ($1, $2, '2026-08-08') RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .fetch_one(&owner_actor)
    .await?;

    MIGRATOR.run_to(32, owner).await?;
    assert_eq!(applied_count(owner).await?, 32);
    let worker = role_pool(super_url, db, "worker").await?;
    let partial_control_active: bool = sqlx::query_scalar(
        "SELECT active FROM recommendation_scheduler_control \
         WHERE control_key = 'scheduler'",
    )
    .fetch_one(owner)
    .await?;
    assert!(!partial_control_active);
    let worker_execute_before_activation: bool = sqlx::query_scalar(
        "SELECT has_function_privilege( \
         'worker', \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)', \
         'EXECUTE')",
    )
    .fetch_one(owner)
    .await?;
    assert!(!worker_execute_before_activation);
    let partial_worker_call = sqlx::query(
        "SELECT * FROM schedule_recommendation_run( \
         $1, $2, '2026-08-11', $3, $4, 7, 'not-yet-active')",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&partial_worker_call).as_deref(), Some("42501"));
    let partial_owner_call = sqlx::query(
        "SELECT * FROM schedule_recommendation_run( \
         $1, $2, '2026-08-11', $3, $4, 7, 'not-yet-active')",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&partial_owner_call).as_deref(), Some("55000"));
    assert!(
        partial_owner_call
            .to_string()
            .contains("recommendation scheduler is unavailable")
    );

    MIGRATOR.run_to(33, owner).await?;
    assert_eq!(applied_count(owner).await?, 33);
    let active_control: bool = sqlx::query_scalar(
        "SELECT active FROM recommendation_scheduler_control \
         WHERE control_key = 'scheduler'",
    )
    .fetch_one(owner)
    .await?;
    assert!(active_control);
    MIGRATOR.run_to(38, owner).await?;
    assert_eq!(applied_count(owner).await?, 38);

    let columns: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT table_name, column_name, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND ( \
           (table_name = 'recommendation_runs' AND column_name IN \
             ('job_id', 'trigger_kind', 'dataset_version_id', 'dataset_manifest_sha256')) \
           OR (table_name = 'account_strategy_bindings' \
             AND column_name = 'auto_apply_recommendations') \
           OR (table_name = 'pending_targets' AND column_name IN \
             ('dataset_version_id', 'dataset_manifest_sha256', 'non_execution_reason'))) \
         ORDER BY table_name, column_name",
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(
        columns,
        vec![
            (
                "account_strategy_bindings".into(),
                "auto_apply_recommendations".into(),
                "NO".into(),
                Some("false".into()),
            ),
            (
                "pending_targets".into(),
                "dataset_manifest_sha256".into(),
                "YES".into(),
                None,
            ),
            (
                "pending_targets".into(),
                "dataset_version_id".into(),
                "YES".into(),
                None,
            ),
            (
                "pending_targets".into(),
                "non_execution_reason".into(),
                "YES".into(),
                None,
            ),
            (
                "recommendation_runs".into(),
                "dataset_manifest_sha256".into(),
                "YES".into(),
                None,
            ),
            (
                "recommendation_runs".into(),
                "dataset_version_id".into(),
                "YES".into(),
                None,
            ),
            (
                "recommendation_runs".into(),
                "job_id".into(),
                "YES".into(),
                None,
            ),
            (
                "recommendation_runs".into(),
                "trigger_kind".into(),
                "NO".into(),
                Some("'MANUAL'::text".into()),
            ),
        ],
        "recommendation migrations must preserve old rows while adding explicit lineage"
    );

    let legacy_defaults: (String, Option<Uuid>, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT trigger_kind, job_id, dataset_version_id, dataset_manifest_sha256 \
             FROM recommendation_runs WHERE id = $1",
    )
    .bind(legacy_run_id)
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(legacy_defaults, ("MANUAL".into(), None, None, None));
    let binding_default: bool = sqlx::query_scalar(
        "SELECT auto_apply_recommendations FROM account_strategy_bindings WHERE id = $1",
    )
    .bind(binding_id)
    .fetch_one(&owner_actor)
    .await?;
    assert!(!binding_default, "Paper automation must be opt-in");

    let constraints: Vec<(String, String)> = sqlx::query_as(
        "SELECT conname, pg_get_constraintdef(oid) \
         FROM pg_constraint WHERE conname = ANY($1) ORDER BY conname",
    )
    .bind(
        [
            "recommendation_items_run_instrument_key",
            "recommendation_runs_dataset_manifest_sha256_check",
            "recommendation_runs_dataset_version_id_fkey",
            "recommendation_runs_job_id_fkey",
            "recommendation_runs_scheduled_lineage_check",
            "recommendation_runs_trigger_check",
        ]
        .as_slice(),
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(constraints.len(), 6, "all 0026 constraints must exist");
    let constraint_definition = |name: &str| {
        constraints
            .iter()
            .find(|(constraint, _)| constraint == name)
            .map(|(_, definition)| definition.as_str())
            .expect("expected 0026 constraint")
    };
    assert!(
        constraint_definition("recommendation_runs_job_id_fkey")
            .contains("FOREIGN KEY (job_id) REFERENCES jobs(id)")
    );
    assert!(
        constraint_definition("recommendation_runs_dataset_version_id_fkey")
            .contains("FOREIGN KEY (dataset_version_id) REFERENCES dataset_versions(id)")
    );
    let trigger_check = constraint_definition("recommendation_runs_trigger_check");
    assert!(trigger_check.contains("MANUAL") && trigger_check.contains("SCHEDULED"));
    let manifest_check = constraint_definition("recommendation_runs_dataset_manifest_sha256_check");
    assert!(manifest_check.contains("IS NULL") && manifest_check.contains("^[0-9a-f]{64}$"));
    let scheduled_lineage = constraint_definition("recommendation_runs_scheduled_lineage_check");
    for required in [
        "strategy_config_id IS NOT NULL",
        "dataset_version_id IS NOT NULL",
        "dataset_manifest_sha256 IS NOT NULL",
        "job_id IS NOT NULL",
    ] {
        assert!(
            scheduled_lineage.contains(required),
            "scheduled lineage check is missing {required}: {scheduled_lineage}"
        );
    }
    assert!(
        constraint_definition("recommendation_items_run_instrument_key")
            .contains("UNIQUE (recommendation_run_id, instrument_id)")
    );

    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT indexname, indexdef FROM pg_indexes \
         WHERE schemaname = 'public' AND indexname = ANY($1) ORDER BY indexname",
    )
    .bind(
        [
            "jobs_typed_claim_idx",
            "recommendation_runs_job_id_uq",
            "recommendation_runs_scheduled_identity_uq",
            "target_portfolios_one_per_run",
        ]
        .as_slice(),
    )
    .fetch_all(owner)
    .await?;
    assert_eq!(indexes.len(), 4, "all 0026 indexes must exist");
    let index_definition = |name: &str| {
        indexes
            .iter()
            .find(|(index, _)| index == name)
            .map(|(_, definition)| definition.as_str())
            .expect("expected 0026 index")
    };
    let typed_claim = index_definition("jobs_typed_claim_idx");
    assert!(
        typed_claim.contains("(job_type, priority DESC, created_at) INCLUDE (available_at)")
            && typed_claim.contains("WHERE (status = 'QUEUED'::text)")
    );
    for unique_partial in [
        "recommendation_runs_job_id_uq",
        "recommendation_runs_scheduled_identity_uq",
        "target_portfolios_one_per_run",
    ] {
        let definition = index_definition(unique_partial);
        assert!(
            definition.contains("CREATE UNIQUE INDEX") && definition.contains(" WHERE "),
            "{unique_partial} must be a partial unique index: {definition}"
        );
    }

    let function_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        function_metadata.0,
        "scheduler function must be SECURITY DEFINER"
    );
    assert_eq!(function_metadata.1, "migration_owner");
    assert_eq!(
        function_metadata.2,
        Some(vec!["search_path=pg_catalog, public".into()]),
        "scheduler function must pin a safe search_path"
    );
    let publication_lock_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.lock_recommendation_publication_inputs(uuid,uuid,text,text,jsonb,uuid,text,text,text,text,text,text,jsonb)'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(publication_lock_metadata.0);
    assert_eq!(publication_lock_metadata.1, "migration_owner");
    assert_eq!(
        publication_lock_metadata.2,
        Some(vec!["search_path=pg_catalog, pg_temp".into()])
    );
    let entitlement_lock_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.lock_recommendation_entitlement(uuid,text,date)'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(entitlement_lock_metadata.0);
    assert_eq!(entitlement_lock_metadata.1, "migration_owner");
    assert_eq!(
        entitlement_lock_metadata.2,
        Some(vec!["search_path=pg_catalog, pg_temp".into()])
    );
    let source_pin_lock_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.lock_recommendation_source_pins(uuid[],text[],text[])'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(source_pin_lock_metadata.0);
    assert_eq!(source_pin_lock_metadata.1, "migration_owner");
    assert_eq!(
        source_pin_lock_metadata.2,
        Some(vec!["search_path=pg_catalog, pg_temp".into()])
    );
    let terminal_sync: (String, String) = sqlx::query_as(
        "SELECT terminal_trigger.tgname, terminal_fn.proname \
         FROM pg_trigger AS terminal_trigger \
         JOIN pg_proc AS terminal_fn ON terminal_fn.oid = terminal_trigger.tgfoid \
         WHERE terminal_trigger.tgrelid = 'public.jobs'::regclass \
           AND NOT terminal_trigger.tgisinternal \
           AND terminal_trigger.tgname = 'jobs_sync_recommendation_terminal_run'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        terminal_sync,
        (
            "jobs_sync_recommendation_terminal_run".into(),
            "sync_recommendation_run_from_terminal_job".into(),
        )
    );
    let terminal_sync_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = \
         'public.sync_recommendation_run_from_terminal_job()'::regprocedure",
    )
    .fetch_one(owner)
    .await?;
    assert!(terminal_sync_metadata.0);
    assert_eq!(terminal_sync_metadata.1, "migration_owner");
    assert_eq!(
        terminal_sync_metadata.2,
        Some(vec!["search_path=pg_catalog, pg_temp".into()])
    );
    let scheduled_guard: (String, String) = sqlx::query_as(
        "SELECT guard_trigger.tgname, guard_fn.proname FROM pg_trigger AS guard_trigger \
         JOIN pg_proc AS guard_fn ON guard_fn.oid = guard_trigger.tgfoid \
         WHERE guard_trigger.tgrelid = 'public.recommendation_runs'::regclass \
           AND NOT guard_trigger.tgisinternal \
           AND guard_trigger.tgname = 'recommendation_runs_protect_scheduled_lineage'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        scheduled_guard,
        (
            "recommendation_runs_protect_scheduled_lineage".into(),
            "recommendation_runs_reject_scheduled_lineage_mutation".into(),
        )
    );
    let scheduled_job_guard: (String, String) = sqlx::query_as(
        "SELECT guard_trigger.tgname, guard_fn.proname FROM pg_trigger AS guard_trigger \
         JOIN pg_proc AS guard_fn ON guard_fn.oid = guard_trigger.tgfoid \
         WHERE guard_trigger.tgrelid = 'public.jobs'::regclass \
           AND NOT guard_trigger.tgisinternal \
           AND guard_trigger.tgname = 'jobs_protect_scheduled_recommendation_lineage'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        scheduled_job_guard,
        (
            "jobs_protect_scheduled_recommendation_lineage".into(),
            "jobs_reject_scheduled_recommendation_mutation".into(),
        )
    );
    for (role, expected) in [
        ("worker", true),
        ("app", false),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_execute: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_execute, expected,
            "unexpected scheduler EXECUTE privilege for {role}"
        );
    }
    for signature in [
        "public.lock_recommendation_schedule_inputs(date,uuid,text,text,text)",
        "public.lock_recommendation_calendar_coverage(date)",
        "public.queue_scheduled_paper_targets(uuid,uuid,uuid,date,uuid,text,text,jsonb)",
        "public.preflight_paper_target(uuid,uuid)",
        "public.snapshot_paper_rebalance_preview(uuid,uuid,date)",
        "public.publish_paper_rebalance_preview(uuid,uuid,bigint,text,text,integer,date,text,jsonb,jsonb)",
        "public.fail_paper_rebalance_preview(uuid,uuid,jsonb)",
    ] {
        let metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
            "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
             FROM pg_proc WHERE oid = $1::regprocedure",
        )
        .bind(signature)
        .fetch_one(owner)
        .await?;
        assert!(metadata.0, "{signature} must be SECURITY DEFINER");
        assert_eq!(
            metadata.1, "migration_owner",
            "unexpected owner for {signature}"
        );
        assert_eq!(
            metadata.2,
            Some(vec!["search_path=pg_catalog, pg_temp".into()]),
            "unsafe search_path for {signature}"
        );
        for (role, expected) in [
            ("worker", true),
            ("app", false),
            ("admin", false),
            ("audit_writer", false),
            ("research_writer", false),
        ] {
            let can_execute: bool =
                sqlx::query_scalar("SELECT has_function_privilege($1, $2, 'EXECUTE')")
                    .bind(role)
                    .bind(signature)
                    .fetch_one(owner)
                    .await?;
            assert_eq!(
                can_execute, expected,
                "unexpected EXECUTE privilege for {role} on {signature}"
            );
        }
    }
    for role in ["worker", "app", "admin", "audit_writer", "research_writer"] {
        let old_bridge: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.queue_scheduled_paper_targets(uuid,uuid,date,uuid,text,text,jsonb)', \
             'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert!(
            !old_bridge,
            "legacy scheduled bridge must be inactive for {role}"
        );
    }
    let submission_signature =
        "public.lock_paper_rebalance_preview_submission(uuid,uuid,uuid,date)";
    let submission_metadata: (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "SELECT prosecdef, pg_get_userbyid(proowner), proconfig \
         FROM pg_proc WHERE oid = $1::regprocedure",
    )
    .bind(submission_signature)
    .fetch_one(owner)
    .await?;
    assert!(
        submission_metadata.0,
        "submission lock must be SECURITY DEFINER"
    );
    assert_eq!(submission_metadata.1, "migration_owner");
    assert_eq!(
        submission_metadata.2,
        Some(vec!["search_path=pg_catalog, pg_temp".into()])
    );
    for (role, expected) in [
        ("worker", false),
        ("app", true),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_submit: bool =
            sqlx::query_scalar("SELECT has_function_privilege($1, $2, 'EXECUTE')")
                .bind(role)
                .bind(submission_signature)
                .fetch_one(owner)
                .await?;
        assert_eq!(
            can_submit, expected,
            "unexpected preview submission grant for {role}"
        );
    }
    for (role, expected) in [
        ("worker", false),
        ("app", true),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_apply: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.apply_paper_rebalance_preview(uuid,uuid,text,date)', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_apply, expected,
            "unexpected preview apply grant for {role}"
        );
    }
    for role in ["worker", "app", "admin", "audit_writer", "research_writer"] {
        let can_execute: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.sync_recommendation_run_from_terminal_job()', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert!(
            !can_execute,
            "terminal synchronization is trigger-only for {role}"
        );
    }
    for (role, expected) in [
        ("worker", true),
        ("app", false),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_execute: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.lock_recommendation_entitlement(uuid,text,date)', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_execute, expected,
            "unexpected entitlement lock EXECUTE privilege for {role}"
        );
    }
    for (role, expected) in [
        ("worker", true),
        ("app", false),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_execute: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.lock_recommendation_source_pins(uuid[],text[],text[])', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_execute, expected,
            "unexpected source pin lock EXECUTE privilege for {role}"
        );
    }
    for (role, expected) in [
        ("worker", true),
        ("app", false),
        ("admin", false),
        ("audit_writer", false),
        ("research_writer", false),
    ] {
        let can_execute: bool = sqlx::query_scalar(
            "SELECT has_function_privilege($1, \
             'public.lock_recommendation_publication_inputs(uuid,uuid,text,text,jsonb,uuid,text,text,text,text,text,text,jsonb)', 'EXECUTE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_execute, expected,
            "unexpected publication lock EXECUTE privilege for {role}"
        );
    }

    for (table, expected) in [
        ("recommendation_runs", (true, false, false, false)),
        ("recommendation_items", (true, true, false, false)),
        ("target_portfolios", (true, true, false, false)),
        ("jobs", (true, false, true, false)),
        ("user_strategy_configs", (true, false, false, false)),
        ("account_strategy_bindings", (true, false, false, false)),
        ("pending_targets", (true, false, true, false)),
        (
            "recommendation_scheduler_control",
            (false, false, false, false),
        ),
    ] {
        let privileges: (bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_table_privilege('worker', $1, 'SELECT'), \
                    has_table_privilege('worker', $1, 'INSERT'), \
                    has_table_privilege('worker', $1, 'UPDATE'), \
                    has_table_privilege('worker', $1, 'DELETE')",
        )
        .bind(table)
        .fetch_one(owner)
        .await?;
        assert_eq!(privileges, expected, "unexpected worker grants on {table}");
    }
    let direct_target_insert = sqlx::query(
        "INSERT INTO pending_targets \
         (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, targets_json) \
         VALUES ($1, $2, $3, '2026-08-11', '2026-08-12', '[]'::jsonb)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&direct_target_insert).as_deref(),
        Some("42501"),
        "worker must queue Paper targets only through the guarded function"
    );
    for (column, expected) in [
        ("status", true),
        ("summary_json", true),
        ("owner_user_id", false),
        ("job_id", false),
        ("trigger_kind", false),
        ("strategy_config_id", false),
        ("as_of", false),
        ("dataset_version_id", false),
        ("dataset_manifest_sha256", false),
    ] {
        let can_update: bool = sqlx::query_scalar(
            "SELECT has_column_privilege('worker', 'recommendation_runs', $1, 'UPDATE')",
        )
        .bind(column)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            can_update, expected,
            "unexpected worker UPDATE privilege on recommendation_runs.{column}"
        );
    }

    let app = role_pool(super_url, db, "app").await?;
    let app_function_denied = sqlx::query(
        "SELECT * FROM schedule_recommendation_run($1, $2, '2026-08-11', $3, $4, 7, 'x')",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&app)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&app_function_denied).as_deref(), Some("42501"));

    let expected_key: String = sqlx::query_scalar(
        "SELECT 'recommendation:scheduled:' || \
         md5(concat_ws('|', $1::text, $2::text, to_char($3::date, 'YYYY-MM-DD'), $4::text))",
    )
    .bind(user_id)
    .bind(config_id)
    .bind("2026-08-11")
    .bind(dataset_version_id)
    .fetch_one(owner)
    .await?;
    const SCHEDULE_SQL: &str = "SELECT run_id, job_id FROM schedule_recommendation_run( \
         $1, $2, $3::date, $4, $5, $6, $7)";

    let no_opt_in = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(
        pg_code(&no_opt_in).as_deref(),
        Some("42501"),
        "default-false binding must not authorize scheduling"
    );

    sqlx::query(
        "UPDATE account_strategy_bindings SET auto_apply_recommendations = true WHERE id = $1",
    )
    .bind(binding_id)
    .execute(&owner_actor)
    .await?;

    let missing_weight = sqlx::query(
        "SELECT * FROM public.queue_scheduled_paper_targets(\
            $1, $2, $3, '2026-08-11', $4, '2026-08-11', $5, \
            '[{\"instrument_id\":\"069500.KRX\"}]'::jsonb)",
    )
    .bind(legacy_run_id)
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&missing_weight).as_deref(),
        Some("22023"),
        "the definer boundary must reject a target item with no exact weight"
    );

    let forged_target_lineage = sqlx::query(
        "SELECT * FROM public.queue_scheduled_paper_targets(\
            $1, $2, $3, '2026-08-11', $4, 'forged-version', $5, \
            '[{\"instrument_id\":\"069500.KRX\",\"weight\":\"1.000000\"}]'::jsonb)",
    )
    .bind(legacy_run_id)
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&forged_target_lineage).as_deref(),
        Some("22023"),
        "the definer boundary must attest the target dataset UUID/version/manifest tuple"
    );

    let incomplete_reason = sqlx::query(
        "INSERT INTO pending_targets \
         (account_id, owner_user_id, strategy_config_id, computed_on, effective_date, \
          targets_json, status, executed_at, non_execution_reason) \
         VALUES ($1, $2, $3, '2026-08-20', '2026-08-21', '[]'::jsonb, \
                 'SKIPPED', now(), '{\"other\":\"missing required keys\"}'::jsonb)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&incomplete_reason).as_deref(),
        Some("23514"),
        "structured non-execution reasons must contain code and message"
    );

    let invalid_curated_version = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(0_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&invalid_curated_version).as_deref(), Some("22023"));

    sqlx::query("UPDATE user_strategy_configs SET is_active = false WHERE id = $1")
        .bind(config_id)
        .execute(&owner_actor)
        .await?;
    let inactive_config = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&inactive_config).as_deref(), Some("42501"));
    sqlx::query("UPDATE user_strategy_configs SET is_active = true WHERE id = $1")
        .bind(config_id)
        .execute(&owner_actor)
        .await?;

    sqlx::query("UPDATE accounts SET status = 'SUSPENDED' WHERE id = $1")
        .bind(account_id)
        .execute(&owner_actor)
        .await?;
    let inactive_account = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&inactive_account).as_deref(), Some("42501"));
    sqlx::query("UPDATE accounts SET status = 'ACTIVE' WHERE id = $1")
        .bind(account_id)
        .execute(&owner_actor)
        .await?;

    sqlx::query("UPDATE dataset_versions SET status = 'BLOCKED' WHERE id = $1")
        .bind(dataset_version_id)
        .execute(owner)
        .await?;
    let blocked_dataset = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&blocked_dataset).as_deref(), Some("22023"));
    sqlx::query("UPDATE dataset_versions SET status = 'WARNING' WHERE id = $1")
        .bind(dataset_version_id)
        .execute(owner)
        .await?;

    let foreign_owner = Uuid::parse_str("00000000-0000-0000-0000-000000000026").unwrap();
    let foreign_owner_key: String = sqlx::query_scalar(
        "SELECT 'recommendation:scheduled:' || \
         md5(concat_ws('|', $1::text, $2::text, to_char($3::date, 'YYYY-MM-DD'), $4::text))",
    )
    .bind(foreign_owner)
    .bind(config_id)
    .bind("2026-08-11")
    .bind(dataset_version_id)
    .fetch_one(owner)
    .await?;
    let foreign_owner_denied = sqlx::query(SCHEDULE_SQL)
        .bind(foreign_owner)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(foreign_owner_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&foreign_owner_denied).as_deref(), Some("42501"));

    let bad_key = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind("caller-controlled-key")
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&bad_key).as_deref(), Some("22023"));
    let wrong_manifest = sqlx::query(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("b".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&wrong_manifest).as_deref(), Some("22023"));

    let call_one = sqlx::query_as::<_, (Uuid, Uuid)>(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(expected_key.clone())
        .fetch_one(&worker);
    let call_two = sqlx::query_as::<_, (Uuid, Uuid)>(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(expected_key.clone())
        .fetch_one(&worker);
    let (first, second) = tokio::join!(call_one, call_two);
    let first = first?;
    let second = second?;
    assert_eq!(
        first, second,
        "concurrent scheduling must return one identity"
    );
    let mut dmy_worker = worker.acquire().await?;
    sqlx::query("SET DateStyle TO 'SQL, DMY'")
        .execute(&mut *dmy_worker)
        .await?;
    let dmy_retry: (Uuid, Uuid) = sqlx::query_as(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(expected_key.clone())
        .fetch_one(&mut *dmy_worker)
        .await?;
    assert_eq!(
        dmy_retry, first,
        "DateStyle must not alter scheduler identity"
    );
    drop(dmy_worker);

    let scheduled_counts: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM recommendation_runs WHERE trigger_kind = 'SCHEDULED'), \
           (SELECT count(*) FROM jobs WHERE job_type = 'recommendation')",
    )
    .fetch_one(&worker)
    .await?;
    assert_eq!(scheduled_counts, (1, 1));
    let lineage: (Uuid, String, Uuid, String, Uuid) = sqlx::query_as(
        "SELECT job_id, trigger_kind, dataset_version_id, dataset_manifest_sha256, \
                strategy_config_id \
         FROM recommendation_runs WHERE id = $1",
    )
    .bind(first.0)
    .fetch_one(&worker)
    .await?;
    assert_eq!(lineage.0, first.1);
    assert_eq!(lineage.1, "SCHEDULED");
    assert_eq!(lineage.2, dataset_version_id);
    assert_eq!(lineage.3, "a".repeat(64));
    assert_eq!(lineage.4, config_id);
    let job: (Uuid, String, String, bool) = sqlx::query_as(
        "SELECT owner_user_id, job_type, idempotency_key, \
                payload_json = jsonb_build_object( \
                    'run_id', $2::uuid, \
                    'strategy_config_id', $3::uuid, \
                    'as_of', '2026-08-11', \
                    'dataset', jsonb_build_object( \
                        'id', $4::uuid, \
                        'dataset_id', 'kr-etf-core', \
                        'version', '2026-08-11', \
                        'curated_version', 7, \
                        'manifest_sha256', $5::text \
                    ) \
                ) \
         FROM jobs WHERE id = $1",
    )
    .bind(first.1)
    .bind(first.0)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .fetch_one(&worker)
    .await?;
    assert_eq!(job.0, user_id);
    assert_eq!(job.1, "recommendation");
    assert_eq!(job.2, expected_key);
    assert!(job.3, "scheduled job payload must match Task 3 exactly");

    let app_actor = actor_pool(super_url, db, "app", &user_id.to_string()).await?;
    let forged_scheduled = sqlx::query(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, trigger_kind, dataset_version_id, \
          dataset_manifest_sha256, job_id) \
         VALUES ($1, $2, '2026-08-12', 'SCHEDULED', $3, $4, $5)",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .bind(first.1)
    .execute(&app_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&forged_scheduled).as_deref(), Some("42501"));
    let rewritten_scheduled =
        sqlx::query("UPDATE recommendation_runs SET as_of = '2026-08-12' WHERE id = $1")
            .bind(first.0)
            .execute(&app_actor)
            .await
            .unwrap_err();
    assert_eq!(pg_code(&rewritten_scheduled).as_deref(), Some("42501"));
    let deleted_scheduled = sqlx::query("DELETE FROM recommendation_runs WHERE id = $1")
        .bind(first.0)
        .execute(&app_actor)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&deleted_scheduled).as_deref(), Some("42501"));
    let app_rewrites_job = sqlx::query("UPDATE jobs SET job_type = 'backtest' WHERE id = $1")
        .bind(first.1)
        .execute(&app_actor)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&app_rewrites_job).as_deref(), Some("42501"));
    let worker_rewrites_job =
        sqlx::query("UPDATE jobs SET payload_json = '{}'::jsonb WHERE id = $1")
            .bind(first.1)
            .execute(&worker)
            .await
            .unwrap_err();
    assert_eq!(pg_code(&worker_rewrites_job).as_deref(), Some("42501"));
    let app_reserves_scheduled_key = sqlx::query(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key) \
         VALUES ($1, 'backtest', $2)",
    )
    .bind(user_id)
    .bind(&expected_key)
    .execute(&app_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&app_reserves_scheduled_key).as_deref(),
        Some("42501")
    );
    let ordinary_job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key) \
         VALUES ($1, 'backtest', 'ordinary-backtest') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&app_actor)
    .await?;
    let worker_enters_scheduled_namespace =
        sqlx::query("UPDATE jobs SET idempotency_key = $1 WHERE id = $2")
            .bind(&expected_key)
            .bind(ordinary_job)
            .execute(&worker)
            .await
            .unwrap_err();
    assert_eq!(
        pg_code(&worker_enters_scheduled_namespace).as_deref(),
        Some("42501")
    );

    sqlx::query("UPDATE recommendation_runs SET summary_json = '{\"worker\":true}' WHERE id = $1")
        .bind(first.0)
        .execute(&worker)
        .await?;
    sqlx::query(
        "INSERT INTO recommendation_items \
         (recommendation_run_id, owner_user_id, instrument_id, rank) \
         VALUES ($1, $2, '069500.KRX', 1)",
    )
    .bind(first.0)
    .bind(user_id)
    .execute(&worker)
    .await?;
    let duplicate_item = sqlx::query(
        "INSERT INTO recommendation_items \
         (recommendation_run_id, owner_user_id, instrument_id, rank) \
         VALUES ($1, $2, '069500.KRX', 2)",
    )
    .bind(first.0)
    .bind(user_id)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&duplicate_item).as_deref(), Some("23505"));
    sqlx::query(
        "INSERT INTO target_portfolios \
         (owner_user_id, recommendation_run_id, as_of) VALUES ($1, $2, '2026-08-11')",
    )
    .bind(user_id)
    .bind(first.0)
    .execute(&worker)
    .await?;
    let duplicate_target = sqlx::query(
        "INSERT INTO target_portfolios \
         (owner_user_id, recommendation_run_id, as_of) VALUES ($1, $2, '2026-08-11')",
    )
    .bind(user_id)
    .bind(first.0)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&duplicate_target).as_deref(), Some("23505"));

    for (statement, message) in [
        (
            "INSERT INTO recommendation_runs (owner_user_id, strategy_config_id, as_of) \
             VALUES ('00000000-0000-0000-0000-000000000001', NULL, '2026-08-11')",
            "worker must not insert arbitrary recommendation runs",
        ),
        (
            "INSERT INTO jobs (owner_user_id, job_type) \
             VALUES ('00000000-0000-0000-0000-000000000001', 'recommendation')",
            "worker must not insert arbitrary jobs",
        ),
        (
            "UPDATE user_strategy_configs SET is_active = false",
            "worker must not modify strategy configs",
        ),
        (
            "UPDATE recommendation_runs SET dataset_version_id = NULL",
            "worker must not rewrite recommendation lineage",
        ),
        (
            "UPDATE account_strategy_bindings SET auto_apply_recommendations = false",
            "worker must not manufacture or revoke automation consent",
        ),
        (
            "DELETE FROM recommendation_runs",
            "worker must not delete recommendation runs",
        ),
    ] {
        let denied = sqlx::query(statement).execute(&worker).await.unwrap_err();
        assert_eq!(pg_code(&denied).as_deref(), Some("42501"), "{message}");
    }
    let binding_insert_denied = sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version) \
         VALUES ($1, $2, $3, 'recommendation_contract', '1.0.0')",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&binding_insert_denied).as_deref(), Some("42501"));
    let binding_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM account_strategy_bindings WHERE id = $1")
            .bind(binding_id)
            .fetch_one(&worker)
            .await?;
    assert_eq!(binding_visible, 1, "worker still needs binding SELECT");

    let invalid_trigger = sqlx::query(
        "INSERT INTO recommendation_runs (owner_user_id, strategy_config_id, as_of, trigger_kind) \
         VALUES ($1, $2, '2026-08-12', 'OTHER')",
    )
    .bind(user_id)
    .bind(config_id)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&invalid_trigger).as_deref(), Some("23514"));
    let invalid_manifest = sqlx::query(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, dataset_manifest_sha256) \
         VALUES ($1, $2, '2026-08-12', $3)",
    )
    .bind(user_id)
    .bind(config_id)
    .bind("A".repeat(64))
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&invalid_manifest).as_deref(), Some("23514"));
    let duplicate_job_link = sqlx::query(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, job_id) \
         VALUES ($1, $2, '2026-08-12', $3)",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(first.1)
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&duplicate_job_link).as_deref(), Some("23505"));
    let incomplete_scheduled = sqlx::query(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, trigger_kind, dataset_version_id, \
          dataset_manifest_sha256) \
         VALUES ($1, $2, '2026-08-12', 'SCHEDULED', $3, $4)",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(dataset_version_id)
    .bind("a".repeat(64))
    .execute(&owner_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&incomplete_scheduled).as_deref(), Some("23514"));

    // SQLx 0.9 does not release its session advisory migration lock when an
    // expected down migration fails. Keep that call on a known connection so
    // the test can release the lock before exercising the successful retry.
    let mut guarded_connection = owner.acquire().await?;
    let guarded_down = MIGRATOR
        .undo(&mut *guarded_connection, 25)
        .await
        .unwrap_err();
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *guarded_connection)
        .await?;
    drop(guarded_connection);
    assert_eq!(migrate_pg_code(&guarded_down).as_deref(), Some("55000"));
    assert!(
        guarded_down
            .to_string()
            .contains("recommendation rollback blocked by scheduled recommendation lineage"),
        "rollback guard must return a stable operator-facing error: {guarded_down}"
    );
    assert_eq!(
        applied_count(owner).await?,
        33,
        "0034 may reverse before the 0033 lineage guard refuses the remaining family"
    );
    let publication_lock_after_failed_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.lock_recommendation_publication_inputs(uuid,uuid,text,text,jsonb,uuid,text,text,text,text,text,text,jsonb)')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(publication_lock_after_failed_down.is_none());
    for signature in [
        "public.lock_recommendation_schedule_inputs(date,uuid,text,text,text)",
        "public.lock_recommendation_calendar_coverage(date)",
        "public.queue_scheduled_paper_targets(uuid,uuid,date,uuid,text,text,jsonb)",
        "public.preflight_paper_target(uuid,uuid)",
    ] {
        let function_after_failed_down: Option<String> =
            sqlx::query_scalar("SELECT to_regprocedure($1)::text")
                .bind(signature)
                .fetch_one(owner)
                .await?;
        assert!(
            function_after_failed_down.is_none(),
            "0037 down must remove {signature} before an older rollback guard can fail"
        );
    }
    let pending_after_failed_down: (bool, i64) = sqlx::query_as(
        "SELECT has_table_privilege('worker', 'pending_targets', 'INSERT'), \
                (SELECT count(*) FROM information_schema.columns \
                  WHERE table_schema = 'public' AND table_name = 'pending_targets' \
                    AND column_name IN \
                      ('dataset_version_id','dataset_manifest_sha256','non_execution_reason'))",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(pending_after_failed_down, (true, 0));
    let function_after_failed_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(function_after_failed_down.is_some());
    let rollback_guard_after_failed_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.assert_no_scheduled_recommendation_lineage()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(rollback_guard_after_failed_down.is_some());
    for index in [
        "public.jobs_typed_claim_idx",
        "public.recommendation_runs_job_id_uq",
        "public.recommendation_runs_scheduled_identity_uq",
        "public.recommendation_items_run_instrument_key",
        "public.target_portfolios_one_per_run",
    ] {
        let remaining: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(index)
            .fetch_one(owner)
            .await?;
        assert!(
            remaining.is_some(),
            "rollback refusal must preserve {index}"
        );
    }
    let constraint_after_failed_down: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint \
         WHERE conname = 'recommendation_items_run_instrument_key'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(constraint_after_failed_down, 1);
    let grants_after_failed_down: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT \
           has_function_privilege( \
             'worker', \
             'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)', \
             'EXECUTE'), \
           has_table_privilege('worker', 'recommendation_runs', 'SELECT'), \
           has_column_privilege('worker', 'recommendation_runs', 'status', 'UPDATE'), \
           has_table_privilege('worker', 'recommendation_items', 'INSERT')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(grants_after_failed_down, (true, true, true, true));
    let active_after_failed_down: bool = sqlx::query_scalar(
        "SELECT active FROM recommendation_scheduler_control \
         WHERE control_key = 'scheduler'",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        active_after_failed_down,
        "failed rollback must restore scheduler activation"
    );
    let lineage_after_failed_down: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM recommendation_runs WHERE id = $1), \
           (SELECT count(*) FROM jobs WHERE id = $2)",
    )
    .bind(first.0)
    .bind(first.1)
    .fetch_one(&worker)
    .await?;
    assert_eq!(lineage_after_failed_down, (1, 1));
    let replay_after_failed_down: (Uuid, Uuid) = sqlx::query_as(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(&expected_key)
        .fetch_one(&worker)
        .await?;
    assert_eq!(replay_after_failed_down, first);

    sqlx::query("DELETE FROM target_portfolios WHERE recommendation_run_id = $1")
        .bind(first.0)
        .execute(&owner_actor)
        .await?;
    sqlx::query("DELETE FROM recommendation_runs WHERE id = $1")
        .bind(first.0)
        .execute(&owner_actor)
        .await?;
    sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(first.1)
        .execute(&owner_actor)
        .await?;

    MIGRATOR.undo(owner, 25).await?;
    assert_eq!(
        applied_count(owner).await?,
        25,
        "0034 through 0026 must reverse in dependency order"
    );
    let columns_after_down: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = 'public' AND ( \
           (table_name = 'recommendation_runs' AND column_name IN \
             ('job_id', 'trigger_kind', 'dataset_version_id', 'dataset_manifest_sha256')) \
           OR (table_name = 'account_strategy_bindings' \
             AND column_name = 'auto_apply_recommendations'))",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(columns_after_down, 0);
    let control_after_down: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.recommendation_scheduler_control')::text")
            .fetch_one(owner)
            .await?;
    assert!(control_after_down.is_none());
    let function_after_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(function_after_down.is_none());
    let guard_after_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.recommendation_runs_reject_scheduled_lineage_mutation()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(guard_after_down.is_none());
    let job_guard_after_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.jobs_reject_scheduled_recommendation_mutation()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(job_guard_after_down.is_none());
    let rollback_guard_after_down: Option<String> = sqlx::query_scalar(
        "SELECT to_regprocedure( \
         'public.assert_no_scheduled_recommendation_lineage()')::text",
    )
    .fetch_one(owner)
    .await?;
    assert!(rollback_guard_after_down.is_none());
    for index in [
        "public.jobs_typed_claim_idx",
        "public.recommendation_runs_job_id_uq",
        "public.recommendation_runs_scheduled_identity_uq",
        "public.target_portfolios_one_per_run",
    ] {
        let remaining: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(index)
            .fetch_one(owner)
            .await?;
        assert!(remaining.is_none(), "0026 down must drop {index}");
    }
    let remaining_item_constraint: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint \
         WHERE conname = 'recommendation_items_run_instrument_key'",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(remaining_item_constraint, 0);
    for (table, expected) in [
        ("recommendation_runs", (false, false, false, false)),
        ("recommendation_items", (false, false, false, false)),
        ("target_portfolios", (false, false, false, false)),
        ("account_strategy_bindings", (true, true, true, false)),
        ("pending_targets", (true, true, true, false)),
    ] {
        let privileges: (bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_table_privilege('worker', $1, 'SELECT'), \
                    has_table_privilege('worker', $1, 'INSERT'), \
                    has_table_privilege('worker', $1, 'UPDATE'), \
                    has_table_privilege('worker', $1, 'DELETE')",
        )
        .bind(table)
        .fetch_one(owner)
        .await?;
        assert_eq!(privileges, expected, "0026 down grant drift on {table}");
    }

    MIGRATOR.run(owner).await?;
    assert_eq!(applied_count(owner).await?, up_migration_count() as i64);
    let reinstalled_pending_grants: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege('worker', 'pending_targets', 'SELECT'), \
                has_table_privilege('worker', 'pending_targets', 'INSERT'), \
                has_table_privilege('worker', 'pending_targets', 'UPDATE')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(reinstalled_pending_grants, (true, false, true));
    sqlx::query(
        "UPDATE account_strategy_bindings SET auto_apply_recommendations = true WHERE id = $1",
    )
    .bind(binding_id)
    .execute(&owner_actor)
    .await?;
    let rescheduled: (Uuid, Uuid) = sqlx::query_as(SCHEDULE_SQL)
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-11")
        .bind(dataset_version_id)
        .bind("a".repeat(64))
        .bind(7_i32)
        .bind(expected_key)
        .fetch_one(&worker)
        .await?;
    assert_ne!(rescheduled, first);
    Ok(())
}

#[tokio::test]
async fn recommendation_scheduler_deactivation_fences_pre_authorized_call() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = recommendation_scheduler_deactivation_fence_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("recommendation scheduler fence contract FAILED: {error}");
    }
}

async fn recommendation_scheduler_deactivation_fence_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run(owner).await?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'fence-owner', 'fence@example.test') RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('recommendation_fence', 'Recommendation Fence', 'Paper')",
    )
    .execute(owner)
    .await?;
    let owner_actor = actor_pool(super_url, db, "migration_owner", &user_id.to_string()).await?;
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'recommendation_fence', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let dataset_version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('fence-dataset', '2026-08-12', 'READY', $1, 'curated/fence') RETURNING id",
    )
    .bind("d".repeat(64))
    .fetch_one(owner)
    .await?;
    let account_id: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (owner_user_id, account_type, name, status) \
         VALUES ($1, 'PAPER', 'fence-paper', 'ACTIVE') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    sqlx::query(
        "INSERT INTO account_strategy_bindings \
         (account_id, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
          auto_apply_recommendations) \
         VALUES ($1, $2, $3, 'recommendation_fence', '1.0.0', true)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(config_id)
    .execute(&owner_actor)
    .await?;
    let expected_key: String = sqlx::query_scalar(
        "SELECT 'recommendation:scheduled:' || \
         md5(concat_ws('|', $1::text, $2::text, to_char($3::date, 'YYYY-MM-DD'), $4::text))",
    )
    .bind(user_id)
    .bind(config_id)
    .bind("2026-08-12")
    .bind(dataset_version_id)
    .fetch_one(owner)
    .await?;
    let worker = role_pool(super_url, db, "worker").await?;
    let observer = admin_pool(&supervisor_db_url(super_url, db)).await?;

    let mut blocker = owner.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1, $2)")
        .bind(RECOMMENDATION_FENCE_LOCK_CLASS)
        .bind(RECOMMENDATION_FENCE_LOCK_OBJECT)
        .execute(&mut *blocker)
        .await?;

    let undo_pool = effective_role_pool(super_url, db, "migration_owner", None, 1).await?;
    let undo_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&undo_pool)
        .await?;
    let undo_task_pool = undo_pool.clone();
    let undo_task = tokio::spawn(async move { MIGRATOR.undo(&undo_task_pool, 32).await });
    wait_for_advisory_wait(&observer, undo_pid, "0033 down").await?;

    let mut worker_connection = worker.acquire().await?;
    let worker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *worker_connection)
        .await?;
    let call_task = tokio::spawn(async move {
        sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT run_id, job_id FROM schedule_recommendation_run( \
             $1, $2, $3::date, $4, $5, $6, $7)",
        )
        .bind(user_id)
        .bind(config_id)
        .bind("2026-08-12")
        .bind(dataset_version_id)
        .bind("d".repeat(64))
        .bind(9_i32)
        .bind(expected_key)
        .fetch_one(&mut *worker_connection)
        .await
    });
    wait_for_advisory_wait(&observer, worker_pid, "pre-authorized worker call").await?;

    blocker.commit().await?;
    tokio::time::timeout(Duration::from_secs(10), undo_task).await???;
    let worker_call = tokio::time::timeout(Duration::from_secs(10), call_task).await??;
    let worker_error = worker_call.unwrap_err();
    assert_eq!(pg_code(&worker_error).as_deref(), Some("55000"));
    assert!(
        worker_error
            .to_string()
            .contains("recommendation scheduler is unavailable")
    );
    assert_eq!(applied_count(owner).await?, 32);
    let deactivated: bool = sqlx::query_scalar(
        "SELECT NOT active FROM recommendation_scheduler_control \
         WHERE control_key = 'scheduler'",
    )
    .fetch_one(owner)
    .await?;
    assert!(deactivated);
    let worker_execute: bool = sqlx::query_scalar(
        "SELECT has_function_privilege( \
         'worker', \
         'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)', \
         'EXECUTE')",
    )
    .fetch_one(owner)
    .await?;
    assert!(!worker_execute);
    let scheduled_rows: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM recommendation_runs WHERE trigger_kind = 'SCHEDULED'), \
           (SELECT count(*) FROM jobs WHERE idempotency_key LIKE 'recommendation:scheduled:%')",
    )
    .fetch_one(&owner_actor)
    .await?;
    assert_eq!(scheduled_rows, (0, 0));

    MIGRATOR.run_to(33, owner).await?;
    Ok(())
}

#[tokio::test]
async fn recommendation_pipeline_index_preflights_reject_duplicates() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = recommendation_pipeline_index_preflight_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("recommendation index preflight contract FAILED: {error}");
    }
}

fn assert_index_preflight_failure(error: &sqlx::Error, index: &str) {
    assert_eq!(pg_code(error).as_deref(), Some("23505"));
    let database_error = error
        .as_database_error()
        .expect("duplicate preflight must be a structured database error");
    assert_eq!(database_error.constraint(), Some(index));
}

async fn execute_up_migration_sql(owner: &PgPool, version: i64) -> Result<(), sqlx::Error> {
    let migration = MIGRATOR
        .migrations
        .iter()
        .find(|migration| {
            migration.version == version
                && migration.migration_type != MigrationType::ReversibleDown
        })
        .expect("tracked up migration");
    sqlx::raw_sql(migration.sql.clone()).execute(owner).await?;
    Ok(())
}

async fn recommendation_pipeline_index_preflight_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    MIGRATOR.run_to(26, owner).await?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://issuer.test', 'index-preflight', 'index-preflight@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('index_preflight', 'Index Preflight', 'Paper')",
    )
    .execute(owner)
    .await?;
    let owner_actor = actor_pool(super_url, db, "migration_owner", &user_id.to_string()).await?;
    let config_id: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'index_preflight', '1.0.0', '{}'::jsonb) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let dataset_version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO dataset_versions \
         (dataset_id, version, status, manifest_sha256, storage_path) \
         VALUES ('index-preflight', '1', 'READY', $1, 'curated/index-preflight') \
         RETURNING id",
    )
    .bind("c".repeat(64))
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('index-preflight.KRX', 'index-preflight', 'KRX', 'KRW')",
    )
    .execute(owner)
    .await?;

    let shared_job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key) \
         VALUES ($1, 'recommendation', 'index-preflight-shared') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let _first_manual_run: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, job_id) \
         VALUES ($1, $2, '2026-08-01', $3) RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(shared_job_id)
    .fetch_one(&owner_actor)
    .await?;
    let duplicate_manual_run: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, job_id) \
         VALUES ($1, $2, '2026-08-02', $3) RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(shared_job_id)
    .fetch_one(&owner_actor)
    .await?;
    let job_index_error = execute_up_migration_sql(owner, 27).await.unwrap_err();
    assert_index_preflight_failure(&job_index_error, "recommendation_runs_job_id_uq");
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS recommendation_runs_job_id_uq")
        .execute(owner)
        .await?;
    sqlx::query("DELETE FROM recommendation_runs WHERE id = $1")
        .bind(duplicate_manual_run)
        .execute(&owner_actor)
        .await?;
    execute_up_migration_sql(owner, 27).await?;

    let scheduled_job_one: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key) \
         VALUES ($1, 'recommendation', 'index-preflight-scheduled-1') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let scheduled_job_two: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, idempotency_key) \
         VALUES ($1, 'recommendation', 'index-preflight-scheduled-2') RETURNING id",
    )
    .bind(user_id)
    .fetch_one(&owner_actor)
    .await?;
    let scheduled_run_one: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, \
          dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, '2026-08-03', 'PENDING', $3, 'SCHEDULED', $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(scheduled_job_one)
    .bind(dataset_version_id)
    .bind("c".repeat(64))
    .fetch_one(&owner_actor)
    .await?;
    let scheduled_run_two: Uuid = sqlx::query_scalar(
        "INSERT INTO recommendation_runs \
         (owner_user_id, strategy_config_id, as_of, status, job_id, trigger_kind, \
          dataset_version_id, dataset_manifest_sha256) \
         VALUES ($1, $2, '2026-08-03', 'PENDING', $3, 'SCHEDULED', $4, $5) RETURNING id",
    )
    .bind(user_id)
    .bind(config_id)
    .bind(scheduled_job_two)
    .bind(dataset_version_id)
    .bind("c".repeat(64))
    .fetch_one(&owner_actor)
    .await?;
    let identity_index_error = execute_up_migration_sql(owner, 28).await.unwrap_err();
    assert_index_preflight_failure(
        &identity_index_error,
        "recommendation_runs_scheduled_identity_uq",
    );
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS recommendation_runs_scheduled_identity_uq")
        .execute(owner)
        .await?;
    sqlx::query("DELETE FROM recommendation_runs WHERE id = $1")
        .bind(scheduled_run_two)
        .execute(&owner_actor)
        .await?;
    sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(scheduled_job_two)
        .execute(&owner_actor)
        .await?;
    execute_up_migration_sql(owner, 28).await?;

    sqlx::query(
        "INSERT INTO recommendation_items \
         (recommendation_run_id, owner_user_id, instrument_id, rank) \
         VALUES ($1, $2, 'index-preflight.KRX', 1), \
                ($1, $2, 'index-preflight.KRX', 2)",
    )
    .bind(scheduled_run_one)
    .bind(user_id)
    .execute(&owner_actor)
    .await?;
    let item_index_error = execute_up_migration_sql(owner, 29).await.unwrap_err();
    assert_index_preflight_failure(&item_index_error, "recommendation_items_run_instrument_key");
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS recommendation_items_run_instrument_key")
        .execute(owner)
        .await?;
    sqlx::query("DELETE FROM recommendation_items WHERE rank = 2")
        .execute(&owner_actor)
        .await?;
    execute_up_migration_sql(owner, 29).await?;
    execute_up_migration_sql(owner, 30).await?;

    sqlx::query(
        "INSERT INTO target_portfolios (owner_user_id, recommendation_run_id, as_of) \
         VALUES ($1, $2, '2026-08-03'), ($1, $2, '2026-08-03')",
    )
    .bind(user_id)
    .bind(scheduled_run_one)
    .execute(&owner_actor)
    .await?;
    let target_index_error = execute_up_migration_sql(owner, 31).await.unwrap_err();
    assert_index_preflight_failure(&target_index_error, "target_portfolios_one_per_run");
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS target_portfolios_one_per_run")
        .execute(owner)
        .await?;
    sqlx::query(
        "DELETE FROM target_portfolios WHERE id = ( \
         SELECT id FROM target_portfolios WHERE recommendation_run_id = $1 LIMIT 1)",
    )
    .bind(scheduled_run_one)
    .execute(&owner_actor)
    .await?;
    execute_up_migration_sql(owner, 31).await?;
    execute_up_migration_sql(owner, 32).await?;
    for index in [
        "public.recommendation_runs_job_id_uq",
        "public.recommendation_runs_scheduled_identity_uq",
        "public.recommendation_items_run_instrument_key",
        "public.target_portfolios_one_per_run",
        "public.jobs_typed_claim_idx",
    ] {
        let present: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(index)
            .fetch_one(owner)
            .await?;
        assert!(present.is_some(), "preflight cleanup must allow {index}");
    }
    Ok(())
}
