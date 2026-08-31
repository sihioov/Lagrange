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
    PostgresInviteStore, PostgresSessionStore, PostgresUserStore, with_authenticated_actor,
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
const CANDIDATE_SOURCE_UP_SQL: &str =
    include_str!("../../../../migrations/0042_candidate_source_contracts.up.sql");
const CANDIDATE_SOURCE_DOWN_SQL: &str =
    include_str!("../../../../migrations/0042_candidate_source_contracts.down.sql");
const CANDIDATE_ANALYSIS_UP_SQL: &str =
    include_str!("../../../../migrations/0043_candidate_analysis_surfaces.up.sql");
const CANDIDATE_ANALYSIS_DOWN_SQL: &str =
    include_str!("../../../../migrations/0043_candidate_analysis_surfaces.down.sql");
const CANDIDATE_PIPELINE_UP_SQL: &str =
    include_str!("../../../../migrations/0044_candidate_pipeline.up.sql");
const CANDIDATE_PIPELINE_DOWN_SQL: &str =
    include_str!("../../../../migrations/0044_candidate_pipeline.down.sql");
const CANDIDATE_MULTI_UNIVERSE_UP_SQL: &str =
    include_str!("../../../../migrations/0045_candidate_multi_universe.up.sql");
const CANDIDATE_MULTI_UNIVERSE_DOWN_SQL: &str =
    include_str!("../../../../migrations/0045_candidate_multi_universe.down.sql");
const CANDIDATE_PRICE_REVALIDATION_UP_SQL: &str =
    include_str!("../../../../migrations/0046_candidate_price_rights_revalidation.up.sql");
const CANDIDATE_PRICE_REVALIDATION_DOWN_SQL: &str =
    include_str!("../../../../migrations/0046_candidate_price_rights_revalidation.down.sql");
const CANDIDATE_WORKER_PRICE_ATTESTATION_UP_SQL: &str = include_str!(
    "../../../../migrations/0047_candidate_worker_price_entitlement_attestation.up.sql"
);
const CANDIDATE_WORKER_PRICE_ATTESTATION_DOWN_SQL: &str = include_str!(
    "../../../../migrations/0047_candidate_worker_price_entitlement_attestation.down.sql"
);
const OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL: &str =
    include_str!("../../../../migrations/0049_owner_beta_price_recommendations.up.sql");
const OWNER_BETA_PRICE_RECOMMENDATIONS_DOWN_SQL: &str =
    include_str!("../../../../migrations/0049_owner_beta_price_recommendations.down.sql");
const OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL: &str =
    include_str!("../../../../migrations/0050_owner_beta_strategy_snapshots.up.sql");
const OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL: &str =
    include_str!("../../../../migrations/0050_owner_beta_strategy_snapshots.down.sql");
const OWNER_BETA_TARGET_PUBLICATION_UP_SQL: &str =
    include_str!("../../../../migrations/0051_owner_beta_target_publication.up.sql");
const OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL: &str =
    include_str!("../../../../migrations/0051_owner_beta_target_publication.down.sql");
const OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL: &str =
    include_str!("../../../../migrations/0052_owner_beta_strategy_config_lock.up.sql");
const OWNER_BETA_STRATEGY_CONFIG_LOCK_DOWN_SQL: &str =
    include_str!("../../../../migrations/0052_owner_beta_strategy_config_lock.down.sql");
const OWNER_EQUITY_UNIVERSE_V2_UP_SQL: &str =
    include_str!("../../../../migrations/0053_owner_managed_equity_universe_v2.up.sql");
const OWNER_EQUITY_UNIVERSE_V2_DOWN_SQL: &str =
    include_str!("../../../../migrations/0053_owner_managed_equity_universe_v2.down.sql");
const CANDIDATE_SCHEDULE_RS: &str =
    include_str!("../../../../crates/job-queue/src/candidate/schedule.rs");
const CANDIDATE_RUNNER_RS: &str =
    include_str!("../../../../crates/job-queue/src/candidate/runner.rs");
const RESEARCH_WORKER_RS: &str =
    include_str!("../../../../data-pipelines/collectors/src/worker.rs");

#[test]
fn owner_beta_price_recommendation_persistence_is_separate_and_fail_closed() {
    assert_eq!(
        MIGRATOR
            .migrations
            .iter()
            .filter(|migration| migration.version == 49)
            .count(),
        2,
        "0049 must have exactly one reversible up/down migration pair"
    );

    for token in [
        "SET LOCAL lock_timeout = '5s';",
        "SET LOCAL statement_timeout = '30s';",
        "CREATE TABLE public.owner_beta_recommendation_runs",
        "CREATE TABLE public.owner_beta_recommendation_items",
        "owner_beta_recommendation_runs_id_owner_key",
        "FOREIGN KEY (recommendation_run_id, owner_user_id)",
        "UNIQUE (recommendation_run_id, instrument_id)",
        "input_kind = 'owner_beta_historical_price_only_v1'",
        "capability = 'PRICE_RETURN_ONLY'",
        "audience = 'OWNER_ONLY'",
        "vendor_snapshot",
        "NOT strict_pit",
        "status IN ('PENDING', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELED')",
        "status <> 'SUCCEEDED' OR factor_snapshot_sha256 IS NOT NULL",
        "error_code text",
        "ALTER TABLE public.owner_beta_recommendation_runs OWNER TO migration_owner",
        "ALTER TABLE public.owner_beta_recommendation_items OWNER TO migration_owner",
        "ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY",
        "ALTER TABLE public.owner_beta_recommendation_items FORCE ROW LEVEL SECURITY",
        "REVOKE ALL ON TABLE public.owner_beta_recommendation_runs",
        "REVOKE ALL ON TABLE public.owner_beta_recommendation_items",
        "CREATE POLICY owner_beta_recommendation_runs_app_insert",
        "CREATE POLICY owner_beta_recommendation_runs_owner_all",
        "CREATE POLICY owner_beta_recommendation_items_owner_all",
        "GRANT INSERT (",
        ") ON public.owner_beta_recommendation_runs TO app;",
        "GRANT UPDATE (",
        ") ON public.owner_beta_recommendation_runs TO worker;",
        ") ON public.owner_beta_recommendation_items TO worker;",
        "FOR UPDATE TO worker",
        "FOR INSERT TO worker",
        "GRANT SELECT ON TABLE public.owner_beta_recommendation_runs TO app, worker, admin",
        "GRANT SELECT ON TABLE public.owner_beta_recommendation_items TO app, worker, admin",
        "CREATE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()",
        "CREATE FUNCTION public.jobs_protect_owner_beta_recommendation_lineage()",
        "BEFORE UPDATE OR DELETE ON public.jobs",
        "owner beta recommendation run identity is immutable",
        "owner beta recommendation job lineage is immutable",
        "v_job_type IS DISTINCT FROM 'owner_beta_price_recommendation'",
        "pg_catalog.jsonb_object_keys(v_payload)",
        "pg_catalog.jsonb_object_keys(v_payload -> 'pins')",
        "'as_of', 'pins', 'run_id', 'strategy_config_id'",
        "'action_manifest_sha256'",
        "'approval_registry_sha256'",
        "'artifact_manifest_sha256'",
        "'candidate_content_sha256'",
        "'stage5_manifest_sha256'",
        "v_payload ->> 'run_id' IS DISTINCT FROM NEW.id::text",
        "v_payload ->> 'strategy_config_id' IS DISTINCT FROM NEW.strategy_config_id::text",
        "v_payload ->> 'as_of' IS DISTINCT FROM pg_catalog.to_char(NEW.as_of, 'YYYY-MM-DD')",
        "NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key",
        "NEW.payload_json IS DISTINCT FROM OLD.payload_json",
    ] {
        assert!(
            OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL.contains(token),
            "0049 owner-beta persistence contract is missing {token}"
        );
    }

    for hash_column in [
        "candidate_content_sha256",
        "artifact_manifest_sha256",
        "stage5_manifest_sha256",
        "action_manifest_sha256",
        "approval_registry_sha256",
    ] {
        assert!(
            OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL
                .contains(&format!("{hash_column} ~ '^sha256:[0-9a-f]{{64}}$'")),
            "0049 must strictly validate {hash_column}"
        );
        assert!(
            OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL
                .contains(&format!("v_payload -> 'pins' ->> '{hash_column}'")),
            "0049 payload binding must compare {hash_column}"
        );
    }
    assert!(
        OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL
            .contains("factor_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'"),
        "0049 must strictly validate the optional factor snapshot hash"
    );
    for deferred_snapshot_field in [
        "strategy_id text",
        "strategy_version text",
        "strategy_config_json",
        "strategy_config_sha256",
        "v_payload -> 'strategy'",
    ] {
        assert!(
            !OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL.contains(deferred_snapshot_field),
            "0049 checksum-era contract must not contain 0050 field {deferred_snapshot_field}"
        );
    }

    let executable_up = OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL.to_ascii_lowercase();
    for forbidden in [
        "dataset_versions",
        "target_portfolios",
        "accounts",
        "paper",
        "curated",
        "ready",
        "error_message",
        "request_path",
        "raw_batch",
        "registration",
        "publication",
    ] {
        assert!(
            !executable_up.contains(forbidden),
            "0049 executable up SQL must not reference {forbidden}"
        );
    }
    for grant in executable_up
        .split(';')
        .map(str::trim)
        .filter(|statement| statement.starts_with("grant "))
    {
        assert!(
            !grant.contains("audit_writer") && !grant.contains("research_writer"),
            "0049 must grant no capability to non-serving writers: {grant}"
        );
        assert!(
            !(grant.starts_with("grant insert")
                && grant.contains("owner_beta_recommendation_items")
                && grant.contains(" to app"))
                && !(grant.starts_with("grant update")
                    && grant.contains("owner_beta_recommendation_runs")
                    && grant.contains(" to app")),
            "0049 must preserve the narrow app write boundary: {grant}"
        );
    }
    assert!(
        !executable_up.contains("delete on table public.owner_beta")
            && !executable_up.contains("truncate on table public.owner_beta"),
        "0049 must grant no destructive table capability"
    );

    for token in [
        "DROP TRIGGER IF EXISTS jobs_protect_owner_beta_recommendation_lineage",
        "DROP TRIGGER IF EXISTS owner_beta_recommendation_runs_validate_job_binding",
        "DROP FUNCTION IF EXISTS public.jobs_protect_owner_beta_recommendation_lineage()",
        "DROP FUNCTION IF EXISTS public.owner_beta_recommendation_runs_validate_job_binding()",
        "DROP POLICY IF EXISTS owner_beta_recommendation_items_owner_all",
        "DROP POLICY IF EXISTS owner_beta_recommendation_items_admin_select",
        "DROP POLICY IF EXISTS owner_beta_recommendation_items_worker_insert",
        "DROP POLICY IF EXISTS owner_beta_recommendation_items_worker_select",
        "DROP POLICY IF EXISTS owner_beta_recommendation_items_app_select",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_owner_all",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_admin_select",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_worker_update",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_worker_select",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_app_insert",
        "DROP POLICY IF EXISTS owner_beta_recommendation_runs_app_select",
        "DROP TABLE IF EXISTS public.owner_beta_recommendation_items",
        "DROP TABLE IF EXISTS public.owner_beta_recommendation_runs",
    ] {
        assert!(
            OWNER_BETA_PRICE_RECOMMENDATIONS_DOWN_SQL.contains(token),
            "0049 rollback is missing {token}"
        );
    }
    assert!(
        !OWNER_BETA_PRICE_RECOMMENDATIONS_DOWN_SQL
            .to_ascii_uppercase()
            .contains("CASCADE"),
        "0049 rollback must not use CASCADE"
    );
}

#[test]
fn owner_beta_strategy_snapshots_are_append_only_and_bound_at_insert() {
    assert_eq!(
        MIGRATOR
            .migrations
            .iter()
            .filter(|migration| migration.version == 50)
            .count(),
        2,
        "0050 must have exactly one reversible up/down migration pair"
    );

    let lock = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;")
        .expect("0050 must lock the run table");
    let no_force = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;")
        .expect("0050 must let the locked table owner inspect every legacy row");
    let guard = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("owner beta strategy snapshot migration requires an empty run table")
        .expect("0050 must fail closed on a legacy owner-beta row");
    let force = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;")
        .expect("0050 must restore forced RLS before changing the schema");
    let schema_change = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs\n    ADD COLUMN strategy_id")
        .expect("0050 must add the strategy snapshot schema");
    assert!(
        lock < no_force && no_force < guard && guard < force && force < schema_change,
        "0050 must lock, inspect every row without tenant GUCs, restore forced RLS, then add snapshot columns"
    );

    for token in [
        "SET LOCAL lock_timeout = '5s';",
        "SET LOCAL statement_timeout = '30s';",
        "LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;",
        "ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;",
        "SELECT 1 FROM public.owner_beta_recommendation_runs",
        "ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;",
        "USING ERRCODE = '55000';",
        "ADD COLUMN strategy_id text NOT NULL",
        "ADD COLUMN strategy_version text NOT NULL",
        "ADD COLUMN strategy_config_json jsonb NOT NULL",
        "ADD COLUMN strategy_config_sha256 text NOT NULL",
        "strategy_id <> ''",
        "strategy_version <> ''",
        "pg_catalog.jsonb_typeof(strategy_config_json) = 'object'",
        "strategy_config_sha256 ~ '^sha256:[0-9a-f]{64}$'",
        "GRANT INSERT (\n    strategy_id, strategy_version, strategy_config_json, strategy_config_sha256\n) ON public.owner_beta_recommendation_runs TO app;",
        "CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()",
        "'as_of', 'pins', 'run_id', 'strategy', 'strategy_config_id'",
        "'config_json', 'config_sha256', 'strategy_id', 'strategy_version'",
        "v_payload -> 'strategy' ->> 'strategy_id'\n            IS DISTINCT FROM NEW.strategy_id",
        "v_payload -> 'strategy' ->> 'strategy_version'\n            IS DISTINCT FROM NEW.strategy_version",
        "v_payload -> 'strategy' -> 'config_json'\n            IS DISTINCT FROM NEW.strategy_config_json",
        "v_payload -> 'strategy' ->> 'config_sha256'\n            IS DISTINCT FROM NEW.strategy_config_sha256",
        "OR NEW.strategy_id IS DISTINCT FROM OLD.strategy_id",
        "OR NEW.strategy_version IS DISTINCT FROM OLD.strategy_version",
        "OR NEW.strategy_config_json IS DISTINCT FROM OLD.strategy_config_json",
        "OR NEW.strategy_config_sha256 IS DISTINCT FROM OLD.strategy_config_sha256",
        "id, owner_user_id, strategy_config_id, strategy_id, strategy_version,\n        strategy_config_json, strategy_config_sha256, job_id, as_of",
    ] {
        assert!(
            OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL.contains(token),
            "0050 owner-beta strategy snapshot contract is missing {token}"
        );
    }
    assert!(
        !OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL.contains("pg_catalog.set_config")
            && !OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL.contains("app.actor_user_id"),
        "0050 migration guards must not contaminate pooled connections with tenant GUCs"
    );

    let update_branch_start = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
        .find("IF TG_OP = 'UPDATE' THEN")
        .expect("0050 must distinguish UPDATE from INSERT");
    let insert_branch_start = OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL[update_branch_start..]
        .find("    ELSE\n        SELECT config.strategy_id")
        .map(|offset| update_branch_start + offset)
        .expect("0050 must attest the current config only on INSERT");
    let update_branch =
        &OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL[update_branch_start..insert_branch_start];
    assert!(
        !update_branch.contains("user_strategy_configs"),
        "0050 UPDATE must not compare an immutable snapshot to the later-mutable config"
    );
    assert_eq!(
        OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
            .matches("FROM public.user_strategy_configs AS config")
            .count(),
        1,
        "0050 must read the current config exactly once, in the INSERT branch"
    );
    for insert_binding in [
        "WHERE config.id = NEW.strategy_config_id",
        "AND config.owner_user_id = NEW.owner_user_id",
        "AND config.is_active",
        "FOR SHARE OF config;",
        "v_config_strategy_id IS DISTINCT FROM NEW.strategy_id",
        "v_config_strategy_version IS DISTINCT FROM NEW.strategy_version",
        "v_config_json IS DISTINCT FROM NEW.strategy_config_json",
    ] {
        assert!(
            OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL[insert_branch_start..].contains(insert_binding),
            "0050 INSERT-time config binding is missing {insert_binding}"
        );
    }
    assert!(
        !OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL.contains("GRANT UPDATE (")
            && !OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL.contains("TO worker"),
        "0050 must not grant snapshot mutation to the worker"
    );
    assert!(
        !OWNER_BETA_STRATEGY_SNAPSHOTS_UP_SQL
            .contains("jobs_protect_owner_beta_recommendation_lineage"),
        "0050 must preserve rather than replace the existing jobs lineage trigger"
    );

    for down_token in [
        "DROP TRIGGER owner_beta_recommendation_runs_validate_job_binding",
        "CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()",
        "'as_of', 'pins', 'run_id', 'strategy_config_id'",
        "PERFORM 1\n      FROM public.user_strategy_configs AS config",
        "id, owner_user_id, strategy_config_id, job_id, as_of",
        "REVOKE INSERT (\n    strategy_id, strategy_version, strategy_config_json, strategy_config_sha256\n) ON public.owner_beta_recommendation_runs FROM app;",
        "DROP COLUMN strategy_config_sha256",
        "DROP COLUMN strategy_config_json",
        "DROP COLUMN strategy_version",
        "DROP COLUMN strategy_id",
    ] {
        assert!(
            OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL.contains(down_token),
            "0050 rollback is missing {down_token}"
        );
    }
    assert!(
        !OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL.contains("v_payload -> 'strategy'")
            && !OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL
                .contains("jobs_protect_owner_beta_recommendation_lineage")
            && !OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL
                .to_ascii_uppercase()
                .contains("CASCADE"),
        "0050 rollback must restore the 0049 payload and preserve job lineage without CASCADE"
    );

    let original_start = OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL
        .find("CREATE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()")
        .expect("0049 binding function");
    let original_end = OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL[original_start..]
        .find("CREATE FUNCTION public.jobs_protect_owner_beta_recommendation_lineage()")
        .map(|offset| original_start + offset)
        .expect("0049 jobs lineage function");
    let restored_start = OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL
        .find("CREATE OR REPLACE FUNCTION public.owner_beta_recommendation_runs_validate_job_binding()")
        .expect("0050 restored binding function");
    let restored_end = OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL[restored_start..]
        .find("REVOKE INSERT (")
        .map(|offset| restored_start + offset)
        .expect("0050 snapshot grant rollback");
    let restored = OWNER_BETA_STRATEGY_SNAPSHOTS_DOWN_SQL[restored_start..restored_end].replacen(
        "CREATE OR REPLACE FUNCTION",
        "CREATE FUNCTION",
        1,
    );
    assert_eq!(
        restored.trim(),
        OWNER_BETA_PRICE_RECOMMENDATIONS_UP_SQL[original_start..original_end].trim(),
        "0050 down must restore the exact 0049-era binding function and trigger"
    );
}

#[test]
fn owner_beta_target_publication_is_atomic_append_only_and_reversible() {
    for (version, description) in [
        (50, "owner beta strategy snapshots"),
        (51, "owner beta target publication"),
    ] {
        let pair = MIGRATOR
            .migrations
            .iter()
            .filter(|migration| migration.version == version)
            .collect::<Vec<_>>();
        assert_eq!(
            pair.len(),
            2,
            "{version:04} must have exactly one reversible up/down migration pair"
        );
        assert!(
            pair.iter()
                .all(|migration| migration.description == description),
            "{version:04} must retain the fixed {description:?} name"
        );
        assert_eq!(
            pair.iter()
                .filter(|migration| migration.migration_type == MigrationType::ReversibleUp)
                .count(),
            1,
            "{version:04} must have exactly one reversible up migration"
        );
        assert_eq!(
            pair.iter()
                .filter(|migration| migration.migration_type == MigrationType::ReversibleDown)
                .count(),
            1,
            "{version:04} must have exactly one reversible down migration"
        );
    }
    let ordered_up = MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.migration_type != MigrationType::ReversibleDown)
        .map(|migration| (migration.version, migration.description.as_ref()))
        .collect::<Vec<_>>();
    assert!(
        ordered_up.windows(2).any(|pair| {
            pair == [
                (50, "owner beta strategy snapshots"),
                (51, "owner beta target publication"),
            ]
        }),
        "0051 must be the exact append-only successor to 0050"
    );

    let lock = OWNER_BETA_TARGET_PUBLICATION_UP_SQL
        .find("LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;")
        .expect("0051 must take the publication table lock");
    let no_force = OWNER_BETA_TARGET_PUBLICATION_UP_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;")
        .expect("0051 must let the locked table owner inspect every legacy row");
    let guard = OWNER_BETA_TARGET_PUBLICATION_UP_SQL
        .find("owner beta target publication migration requires unpublished runs")
        .expect("0051 must reject legacy result state");
    let force = OWNER_BETA_TARGET_PUBLICATION_UP_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;")
        .expect("0051 must restore forced RLS before changing the schema");
    let alter = OWNER_BETA_TARGET_PUBLICATION_UP_SQL
        .find(
            "ALTER TABLE public.owner_beta_recommendation_runs\n    DROP CONSTRAINT owner_beta_recommendation_runs_success_factor_check",
        )
        .expect("0051 must extend the run table");
    assert!(
        lock < no_force && no_force < guard && guard < force && force < alter,
        "0051 must lock, inspect every row without tenant GUCs, restore forced RLS, then alter the run table"
    );

    for token in [
        "SET LOCAL lock_timeout = '5s';",
        "SET LOCAL statement_timeout = '30s';",
        "ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;",
        "run.status = 'SUCCEEDED'",
        "OR run.factor_snapshot_sha256 IS NOT NULL",
        "ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;",
        "USING ERRCODE = '55000';",
        "DROP CONSTRAINT owner_beta_recommendation_runs_success_factor_check",
        "ADD COLUMN target_snapshot_sha256 text",
        "ADD COLUMN cash_weight numeric(18, 6)",
        "target_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'",
        "cash_weight IS NULL OR (cash_weight >= 0 AND cash_weight <= 1)",
        "ADD CONSTRAINT owner_beta_recommendation_runs_result_state_check CHECK",
        "status = 'SUCCEEDED'\n            AND factor_snapshot_sha256 IS NOT NULL\n            AND target_snapshot_sha256 IS NOT NULL\n            AND cash_weight IS NOT NULL\n            AND error_code IS NULL",
        "status <> 'SUCCEEDED'\n            AND factor_snapshot_sha256 IS NULL\n            AND target_snapshot_sha256 IS NULL\n            AND cash_weight IS NULL",
        "GRANT UPDATE (\n    target_snapshot_sha256, cash_weight\n) ON public.owner_beta_recommendation_runs TO worker;",
    ] {
        assert!(
            OWNER_BETA_TARGET_PUBLICATION_UP_SQL.contains(token),
            "0051 target publication contract is missing {token}"
        );
    }
    assert!(
        !OWNER_BETA_TARGET_PUBLICATION_UP_SQL.contains("pg_catalog.set_config")
            && !OWNER_BETA_TARGET_PUBLICATION_UP_SQL.contains("app.actor_user_id"),
        "0051 migration guards must not contaminate pooled connections with tenant GUCs"
    );
    assert_eq!(
        OWNER_BETA_TARGET_PUBLICATION_UP_SQL
            .matches("owner_beta_recommendation_runs_success_factor_check")
            .count(),
        1,
        "0051 up must replace rather than duplicate the 0049 success constraint"
    );

    let executable_up = OWNER_BETA_TARGET_PUBLICATION_UP_SQL.to_ascii_lowercase();
    let grants = executable_up
        .split(';')
        .map(str::trim)
        .filter(|statement| statement.starts_with("grant "))
        .collect::<Vec<_>>();
    assert_eq!(grants.len(), 1, "0051 must add exactly one narrow grant");
    assert_eq!(
        grants[0],
        "grant update (\n    target_snapshot_sha256, cash_weight\n) on public.owner_beta_recommendation_runs to worker",
        "0051 may grant only the two new run result columns to worker"
    );

    let down_lock = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find("LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;")
        .expect("0051 down must take the publication table lock");
    let down_no_force = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;")
        .expect("0051 down must let the locked table owner inspect every result row");
    let down_guard = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find("owner beta target publication rollback would discard lineage")
        .expect("0051 down must preserve published lineage");
    let down_force = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find("ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;")
        .expect("0051 down must restore forced RLS before reverting the schema");
    let down_revoke = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find("REVOKE UPDATE (")
        .expect("0051 down must revoke the new worker grant");
    let down_alter = OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
        .find(
            "ALTER TABLE public.owner_beta_recommendation_runs\n    DROP CONSTRAINT owner_beta_recommendation_runs_result_state_check",
        )
        .expect("0051 down must restore the old schema");
    assert!(
        down_lock < down_no_force
            && down_no_force < down_guard
            && down_guard < down_force
            && down_force < down_revoke
            && down_revoke < down_alter,
        "0051 down must lock, inspect every row without tenant GUCs, restore forced RLS, revoke, then alter"
    );
    for token in [
        "run.target_snapshot_sha256 IS NOT NULL",
        "OR run.cash_weight IS NOT NULL",
        "USING ERRCODE = '55000';",
        "REVOKE UPDATE (\n    target_snapshot_sha256, cash_weight\n) ON public.owner_beta_recommendation_runs FROM worker;",
        "DROP CONSTRAINT owner_beta_recommendation_runs_result_state_check",
        "DROP CONSTRAINT owner_beta_recommendation_runs_cash_weight_check",
        "DROP CONSTRAINT owner_beta_recommendation_runs_target_hash_check",
        "DROP COLUMN cash_weight",
        "DROP COLUMN target_snapshot_sha256",
        "ADD CONSTRAINT owner_beta_recommendation_runs_success_factor_check CHECK (\n        status <> 'SUCCEEDED' OR factor_snapshot_sha256 IS NOT NULL\n    )",
    ] {
        assert!(
            OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL.contains(token),
            "0051 rollback contract is missing {token}"
        );
    }
    assert!(
        !OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL.contains("pg_catalog.set_config")
            && !OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL.contains("app.actor_user_id"),
        "0051 rollback guards must not contaminate pooled connections with tenant GUCs"
    );
    assert!(
        !OWNER_BETA_TARGET_PUBLICATION_UP_SQL
            .to_ascii_uppercase()
            .contains("CASCADE")
            && !OWNER_BETA_TARGET_PUBLICATION_DOWN_SQL
                .to_ascii_uppercase()
                .contains("CASCADE"),
        "0051 must not use CASCADE"
    );
}

#[test]
fn owner_beta_strategy_config_lock_is_lineage_bound_and_reversible() {
    const FUNCTION_NAME: &str =
        "public.owner_beta_recommendation_runs_lock_strategy_config_on_success";
    const TRIGGER_NAME: &str = "owner_beta_recommendation_runs_lock_strategy_config_on_success";
    const DESCRIPTION: &str = "owner beta strategy config lock";

    let version_52 = MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.version == 52)
        .collect::<Vec<_>>();
    assert_eq!(
        version_52.len(),
        2,
        "0052 must have exactly one reversible up/down migration pair"
    );
    assert!(
        version_52
            .iter()
            .all(|migration| migration.description == DESCRIPTION),
        "0052 must retain its fixed migration name"
    );
    assert_eq!(
        version_52
            .iter()
            .filter(|migration| migration.migration_type == MigrationType::ReversibleUp)
            .count(),
        1,
        "0052 must have exactly one reversible up migration"
    );
    assert_eq!(
        version_52
            .iter()
            .filter(|migration| migration.migration_type == MigrationType::ReversibleDown)
            .count(),
        1,
        "0052 must have exactly one reversible down migration"
    );
    let ordered_up = MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.migration_type != MigrationType::ReversibleDown)
        .map(|migration| (migration.version, migration.description.as_ref()))
        .collect::<Vec<_>>();
    assert!(
        ordered_up
            .windows(2)
            .any(|pair| { pair == [(51, "owner beta target publication"), (52, DESCRIPTION),] }),
        "0052 must be the exact append-only successor to 0051"
    );

    assert_eq!(
        OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL
            .matches("CREATE FUNCTION ")
            .count(),
        1,
        "0052 must create exactly one trigger function"
    );
    let function_start = OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL
        .find("CREATE FUNCTION ")
        .expect("0052 CREATE FUNCTION");
    let function_end = OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL[function_start..]
        .find("$lock$;")
        .map(|offset| function_start + offset + "$lock$;".len())
        .expect("0052 complete dollar-quoted trigger function");
    let function = &OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL[function_start..function_end];
    assert_eq!(
        function.matches("AS $lock$").count(),
        1,
        "0052 must have one fixed dollar-quoted function body"
    );
    let expected_header = format!("CREATE FUNCTION {FUNCTION_NAME}()\nRETURNS trigger");
    assert!(
        function.contains(&expected_header),
        "0052 must use the fixed zero-argument trigger-function signature"
    );
    let signature = function
        .split("RETURNS trigger")
        .next()
        .expect("0052 trigger function signature");
    assert_eq!(
        signature
            .split("CREATE FUNCTION ")
            .nth(1)
            .expect("0052 CREATE FUNCTION header")
            .trim(),
        format!("{FUNCTION_NAME}()"),
        "0052 must reject caller parameters and tenant selectors"
    );
    assert!(
        !function.contains("p_owner_user_id")
            && !function.contains("p_strategy_config_id")
            && !function.contains("p_strategy_id")
            && !function.contains("p_strategy_version")
            && !function.contains("p_config_json"),
        "0052 must not restore the rejected caller-selected lock design"
    );
    for token in [
        "SECURITY DEFINER",
        "SET search_path = pg_catalog, pg_temp",
        "NEW.status IS DISTINCT FROM 'SUCCEEDED'",
        "OLD.status IS NOT DISTINCT FROM 'SUCCEEDED'",
        "pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true)",
        "config.id = NEW.strategy_config_id",
        "config.owner_user_id = NEW.owner_user_id",
        "config.is_active",
        "config.strategy_id = NEW.strategy_id",
        "config.strategy_version = NEW.strategy_version",
        "config.config_json = NEW.strategy_config_json",
        "FOR SHARE OF config",
        "USING ERRCODE = '23514'",
    ] {
        assert!(
            function.contains(token),
            "0052 lock function is missing {token}"
        );
    }
    assert_eq!(
        function.matches("pg_catalog.set_config(").count(),
        1,
        "0052 may derive tenant context only once from NEW owner lineage"
    );
    assert!(
        !function.contains("current_setting(")
            && !function.contains("SESSION_USER")
            && !function.contains("CURRENT_USER"),
        "0052 must not accept an ambient or caller-selected tenant identity"
    );

    let top_level_ddl = format!(
        "{}{}",
        &OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL[..function_start],
        &OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL[function_end..]
    );
    let top_level_executable = top_level_ddl
        .lines()
        .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
        .collect::<Vec<_>>()
        .join("\n");
    let up_statements = top_level_executable
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .collect::<Vec<_>>();
    let normalize = |statement: &str| statement.split_whitespace().collect::<Vec<_>>().join(" ");
    let owner_statements = up_statements
        .iter()
        .copied()
        .filter(|statement| statement.contains("ALTER FUNCTION "))
        .map(normalize)
        .collect::<Vec<_>>();
    assert_eq!(
        owner_statements,
        [format!(
            "ALTER FUNCTION {FUNCTION_NAME}() OWNER TO migration_owner"
        )],
        "0052 must assign exactly the fixed trigger function to migration_owner"
    );
    let revoke_statements = up_statements
        .iter()
        .copied()
        .filter(|statement| statement.starts_with("REVOKE "))
        .map(normalize)
        .collect::<Vec<_>>();
    assert_eq!(
        revoke_statements,
        [format!(
            "REVOKE ALL ON FUNCTION {FUNCTION_NAME}() FROM PUBLIC, app, worker, admin, audit_writer, research_writer"
        )],
        "0052 must revoke every direct trigger-function caller"
    );
    let grant_statements = up_statements
        .iter()
        .copied()
        .filter(|statement| statement.starts_with("GRANT "))
        .collect::<Vec<_>>();
    assert!(
        grant_statements.is_empty(),
        "0052 must not broaden worker SELECT, UPDATE, or EXECUTE privileges"
    );

    let trigger_statements = up_statements
        .iter()
        .copied()
        .filter(|statement| statement.contains("CREATE TRIGGER "))
        .map(normalize)
        .collect::<Vec<_>>();
    assert_eq!(
        trigger_statements,
        [format!(
            "CREATE TRIGGER {TRIGGER_NAME} BEFORE UPDATE OF status ON public.owner_beta_recommendation_runs FOR EACH ROW EXECUTE FUNCTION {FUNCTION_NAME}()"
        )],
        "0052 trigger must be limited to owner-beta run status updates"
    );

    let down_drops = OWNER_BETA_STRATEGY_CONFIG_LOCK_DOWN_SQL
        .split(';')
        .map(str::trim)
        .filter(|statement| statement.starts_with("DROP "))
        .map(normalize)
        .collect::<Vec<_>>();
    assert_eq!(
        down_drops,
        [
            format!("DROP TRIGGER {TRIGGER_NAME} ON public.owner_beta_recommendation_runs"),
            format!("DROP FUNCTION {FUNCTION_NAME}()"),
        ],
        "0052 down must drop the exact trigger before the exact function"
    );
    assert!(
        !OWNER_BETA_STRATEGY_CONFIG_LOCK_UP_SQL
            .to_ascii_uppercase()
            .contains("CASCADE")
            && !OWNER_BETA_STRATEGY_CONFIG_LOCK_DOWN_SQL
                .to_ascii_uppercase()
                .contains("CASCADE"),
        "0052 must not use CASCADE"
    );
}

#[test]
fn owner_equity_universe_v2_schema_is_separate_actor_scoped_and_fail_closed() {
    const DESCRIPTION: &str = "owner managed equity universe v2";
    let version_53 = MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.version == 53)
        .collect::<Vec<_>>();
    assert_eq!(
        version_53.len(),
        2,
        "0053 must have exactly one reversible up/down migration pair"
    );
    assert!(
        version_53
            .iter()
            .all(|migration| migration.description == DESCRIPTION),
        "0053 must retain its fixed migration name"
    );
    assert_eq!(
        version_53
            .iter()
            .filter(|migration| migration.migration_type == MigrationType::ReversibleUp)
            .count(),
        1
    );
    assert_eq!(
        version_53
            .iter()
            .filter(|migration| migration.migration_type == MigrationType::ReversibleDown)
            .count(),
        1
    );
    let ordered_up = MIGRATOR
        .migrations
        .iter()
        .filter(|migration| migration.migration_type != MigrationType::ReversibleDown)
        .map(|migration| (migration.version, migration.description.as_ref()))
        .collect::<Vec<_>>();
    assert!(
        ordered_up
            .windows(2)
            .any(|pair| { pair == [(52, "owner beta strategy config lock"), (53, DESCRIPTION),] }),
        "0053 must be the exact append-only successor to 0052"
    );

    for token in [
        "CREATE TABLE public.owner_equity_universe_policies",
        "INSERT INTO public.owner_equity_universe_policies",
        "CREATE FUNCTION public.provision_owner_equity_universe_policy()",
        "CREATE TRIGGER user_roles_provision_owner_equity_universe_policy",
        "max_active_instruments integer NOT NULL",
        "target_observed_sessions integer NOT NULL",
        "minimum_observed_sessions integer NOT NULL",
        "minimum_observed_sessions >= 121",
        "CREATE TABLE public.owner_equity_memberships",
        "owner_equity_memberships_one_active_instrument",
        "WHERE state <> 'DISABLED'",
        "'REQUESTED', 'VALIDATING', 'BACKFILLING', 'MATERIALIZING'",
        "'READY', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'",
        "CREATE TABLE public.owner_equity_membership_events",
        "CREATE TABLE public.owner_equity_instrument_generations",
        "owner equity generation must be monotonically consecutive",
        "CREATE TABLE public.owner_equity_generation_admissions",
        "raw_manifest_sha256 text NOT NULL",
        "artifact_manifest_sha256 text NOT NULL",
        "entitlement_sha256 text NOT NULL",
        "capture_code_commit text NOT NULL",
        "materializer_code_commit text NOT NULL",
        "CREATE TABLE public.owner_equity_signal_snapshots",
        "CREATE TABLE public.owner_equity_signal_snapshot_rows",
        "owner_equity_signal_snapshot_rows_admission_fkey",
        "owner equity signal snapshot universe is not exact",
        "pg_catalog.string_agg(",
        "E'\\n' ORDER BY snapshot_row.instrument_id",
        "owner equity memberships are soft-disabled, never deleted",
        "owner equity lineage is append-only",
        "error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'",
        "error_retryable boolean",
    ] {
        assert!(
            OWNER_EQUITY_UNIVERSE_V2_UP_SQL.contains(token),
            "0053 domain/schema contract is missing {token}"
        );
    }

    for table in [
        "owner_equity_universe_policies",
        "owner_equity_memberships",
        "owner_equity_membership_events",
        "owner_equity_instrument_generations",
        "owner_equity_generation_admissions",
        "owner_equity_signal_snapshots",
        "owner_equity_signal_snapshot_rows",
    ] {
        for token in [
            format!("ALTER TABLE public.{table} OWNER TO migration_owner"),
            format!("ALTER TABLE public.{table} ENABLE ROW LEVEL SECURITY"),
            format!("ALTER TABLE public.{table} FORCE ROW LEVEL SECURITY"),
            format!("REVOKE ALL ON TABLE public.{table}"),
        ] {
            assert!(
                OWNER_EQUITY_UNIVERSE_V2_UP_SQL.contains(&token),
                "0053 RLS/ownership contract is missing {token}"
            );
        }
    }
    for token in [
        "pg_catalog.current_setting('app.actor_user_id', true)",
        "CREATE POLICY owner_equity_memberships_app_select",
        "CREATE POLICY owner_equity_memberships_app_insert",
        "CREATE POLICY owner_equity_memberships_worker_all",
        "CREATE POLICY owner_equity_memberships_admin_select",
        "CREATE POLICY owner_equity_memberships_owner_all",
        "GRANT UPDATE (published_at) ON public.owner_equity_signal_snapshots TO worker",
        "GRANT EXECUTE ON FUNCTION public.retry_owner_equity_membership",
        "GRANT EXECUTE ON FUNCTION public.disable_owner_equity_membership",
        "GRANT EXECUTE ON FUNCTION\n    public.schedule_owner_equity_incremental",
    ] {
        assert!(
            OWNER_EQUITY_UNIVERSE_V2_UP_SQL.contains(token),
            "0053 actor/grant contract is missing {token}"
        );
    }

    let executable_up = OWNER_EQUITY_UNIVERSE_V2_UP_SQL.to_ascii_lowercase();
    for forbidden in [
        "cano",
        "acnt_prdt_cd",
        "kis_account_ref",
        "order_intent",
        "broker_connection",
        "error_message",
        "provider_message",
        "kr-stock-price-beta-v1",
    ] {
        assert!(
            !executable_up.contains(forbidden),
            "0053 must not introduce forbidden/adjacent surface {forbidden}"
        );
    }
    for sql in [
        OWNER_EQUITY_UNIVERSE_V2_UP_SQL,
        OWNER_EQUITY_UNIVERSE_V2_DOWN_SQL,
    ] {
        assert!(
            !sql.to_ascii_uppercase().contains("CASCADE"),
            "0053 must not use CASCADE"
        );
    }
    for token in [
        "IN ACCESS EXCLUSIVE MODE",
        "NO FORCE ROW LEVEL SECURITY",
        "owner equity universe V2 rollback would discard durable state",
        "USING ERRCODE = '55000'",
        "DROP TABLE public.owner_equity_universe_policies",
    ] {
        assert!(
            OWNER_EQUITY_UNIVERSE_V2_DOWN_SQL.contains(token),
            "0053 fail-closed down contract is missing {token}"
        );
    }
}

// TEST-ONLY migration fixtures.  These construct only the public JSON shape
// checked by migrations 0049-0051; they are not production owner-beta input
// constructors and do not represent approved artifact or registry pins.
struct OwnerBetaRuntimeHashes<'a> {
    candidate: &'a str,
    artifact: &'a str,
    stage5: &'a str,
    action: &'a str,
    approval: &'a str,
    strategy: &'a str,
}

fn owner_beta_runtime_payload(
    run_id: Uuid,
    config_id: Uuid,
    hashes: &OwnerBetaRuntimeHashes<'_>,
    strategy_hash: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "run_id": run_id.to_string(),
        "strategy_config_id": config_id.to_string(),
        "as_of": "2026-08-24",
        "pins": {
            "candidate_content_sha256": hashes.candidate,
            "artifact_manifest_sha256": hashes.artifact,
            "stage5_manifest_sha256": hashes.stage5,
            "action_manifest_sha256": hashes.action,
            "approval_registry_sha256": hashes.approval,
        },
    });
    if let Some(strategy_hash) = strategy_hash {
        payload["strategy"] = serde_json::json!({
            "strategy_id": "owner_beta_contract",
            "strategy_version": "1.0.0",
            "config_json": {"benchmark":"069500.KRX"},
            "config_sha256": strategy_hash,
        });
    }
    payload
}

async fn insert_owner_beta_runtime_run(
    app: &PgPool,
    owner_id: Uuid,
    config_id: Uuid,
    run_id: Uuid,
    key: &str,
    hashes: &OwnerBetaRuntimeHashes<'_>,
) -> Result<Uuid, sqlx::Error> {
    let payload = owner_beta_runtime_payload(run_id, config_id, hashes, Some(hashes.strategy));
    let job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, payload_json, idempotency_key) \
         VALUES ($1, 'owner_beta_price_recommendation', $2, $3) RETURNING id",
    )
    .bind(owner_id)
    .bind(payload)
    .bind(key)
    .fetch_one(app)
    .await?;
    sqlx::query(
        "INSERT INTO owner_beta_recommendation_runs \
         (id, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
          strategy_config_json, strategy_config_sha256, job_id, as_of, \
          candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256, \
          action_manifest_sha256, approval_registry_sha256) \
         VALUES ($1, $2, $3, 'owner_beta_contract', '1.0.0', \
                 '{\"benchmark\":\"069500.KRX\"}'::jsonb, $4, $5, '2026-08-24', \
                 $6, $7, $8, $9, $10)",
    )
    .bind(run_id)
    .bind(owner_id)
    .bind(config_id)
    .bind(hashes.strategy)
    .bind(job_id)
    .bind(hashes.candidate)
    .bind(hashes.artifact)
    .bind(hashes.stage5)
    .bind(hashes.action)
    .bind(hashes.approval)
    .execute(app)
    .await?;
    Ok(job_id)
}

#[tokio::test]
async fn owner_beta_migrations_enforce_runtime_rls_lineage_and_rollback_guards() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = owner_beta_migration_runtime_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("owner-beta runtime migration contract FAILED: {error}");
    }
}

async fn owner_beta_migration_runtime_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    // TEST-ONLY synthetic hashes exercise the SQL shape checks.  They are not
    // approval-registry pins or an artifact/publication constructor.
    let test_hash = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
    let candidate_hash = test_hash('a');
    let artifact_hash = test_hash('b');
    let stage5_hash = test_hash('c');
    let action_hash = test_hash('d');
    let approval_hash = test_hash('e');
    let strategy_hash = test_hash('f');
    let factor_hash = test_hash('1');
    let target_hash = test_hash('2');
    let hashes = OwnerBetaRuntimeHashes {
        candidate: &candidate_hash,
        artifact: &artifact_hash,
        stage5: &stage5_hash,
        action: &action_hash,
        approval: &approval_hash,
        strategy: &strategy_hash,
    };

    MIGRATOR.run_to(49, owner).await?;
    assert_eq!(applied_count(owner).await?, 49);

    let owner_a: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://owner-beta.test', 'owner-beta-a', 'owner-beta-a@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    let owner_b: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://owner-beta.test', 'owner-beta-b', 'owner-beta-b@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO strategies (id, display_name, state) \
         VALUES ('owner_beta_contract', 'Owner beta contract', 'Paper')",
    )
    .execute(owner)
    .await?;

    let app_a = actor_pool(super_url, db, "app", &owner_a.to_string()).await?;
    let app_b = actor_pool(super_url, db, "app", &owner_b.to_string()).await?;
    let owner_a_actor = actor_pool(super_url, db, "migration_owner", &owner_a.to_string()).await?;
    let config_a: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'owner_beta_contract', '1.0.0', '{\"benchmark\":\"069500.KRX\"}'::jsonb) \
         RETURNING id",
    )
    .bind(owner_a)
    .fetch_one(&app_a)
    .await?;
    let config_b: Uuid = sqlx::query_scalar(
        "INSERT INTO user_strategy_configs \
         (owner_user_id, strategy_id, strategy_version, config_json) \
         VALUES ($1, 'owner_beta_contract', '1.0.0', '{\"benchmark\":\"069500.KRX\"}'::jsonb) \
         RETURNING id",
    )
    .bind(owner_b)
    .fetch_one(&app_b)
    .await?;

    let legacy_run_id = Uuid::new_v4();
    let legacy_payload = owner_beta_runtime_payload(legacy_run_id, config_a, &hashes, None);
    let legacy_job_id: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, payload_json, idempotency_key) \
         VALUES ($1, 'owner_beta_price_recommendation', $2, 'owner-beta-legacy') RETURNING id",
    )
    .bind(owner_a)
    .bind(&legacy_payload)
    .fetch_one(&app_a)
    .await?;
    sqlx::query(
        "INSERT INTO owner_beta_recommendation_runs \
         (id, owner_user_id, strategy_config_id, job_id, as_of, candidate_content_sha256, \
          artifact_manifest_sha256, stage5_manifest_sha256, action_manifest_sha256, \
          approval_registry_sha256) \
         VALUES ($1, $2, $3, $4, '2026-08-24', $5, $6, $7, $8, $9)",
    )
    .bind(legacy_run_id)
    .bind(owner_a)
    .bind(config_a)
    .bind(legacy_job_id)
    .bind(&candidate_hash)
    .bind(&artifact_hash)
    .bind(&stage5_hash)
    .bind(&action_hash)
    .bind(&approval_hash)
    .execute(&app_a)
    .await?;

    let snapshot_guard = MIGRATOR.run_to(50, owner).await.unwrap_err();
    assert_eq!(migrate_pg_code(&snapshot_guard).as_deref(), Some("55000"));
    assert_eq!(applied_count(owner).await?, 49);
    let legacy_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM owner_beta_recommendation_runs WHERE id = $1")
            .bind(legacy_run_id)
            .fetch_one(&owner_a_actor)
            .await?;
    assert_eq!(legacy_rows, 1, "failed 0050 guard preserves legacy rows");
    sqlx::query("DELETE FROM owner_beta_recommendation_runs WHERE id = $1")
        .bind(legacy_run_id)
        .execute(&owner_a_actor)
        .await?;
    sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(legacy_job_id)
        .execute(&owner_a_actor)
        .await?;

    MIGRATOR.run_to(50, owner).await?;
    assert_eq!(applied_count(owner).await?, 50);

    let run_a = Uuid::new_v4();
    let job_a =
        insert_owner_beta_runtime_run(&app_a, owner_a, config_a, run_a, "owner-beta-a", &hashes)
            .await?;
    let run_b = Uuid::new_v4();
    let _job_b =
        insert_owner_beta_runtime_run(&app_b, owner_b, config_b, run_b, "owner-beta-b", &hashes)
            .await?;

    let mismatched_snapshot_run = Uuid::new_v4();
    let mismatched_snapshot_job: Uuid = sqlx::query_scalar(
        "INSERT INTO jobs (owner_user_id, job_type, payload_json, idempotency_key) \
         VALUES ($1, 'owner_beta_price_recommendation', $2, 'owner-beta-bad-strategy') RETURNING id",
    )
    .bind(owner_a)
    .bind(owner_beta_runtime_payload(
        mismatched_snapshot_run,
        config_a,
        &hashes,
        Some(&strategy_hash),
    ))
    .fetch_one(&app_a)
    .await?;
    let mismatched_snapshot = sqlx::query(
        "INSERT INTO owner_beta_recommendation_runs \
         (id, owner_user_id, strategy_config_id, strategy_id, strategy_version, \
          strategy_config_json, strategy_config_sha256, job_id, as_of, \
          candidate_content_sha256, artifact_manifest_sha256, stage5_manifest_sha256, \
          action_manifest_sha256, approval_registry_sha256) \
         VALUES ($1, $2, $3, 'owner_beta_contract', '9.9.9', $4, $5, $6, '2026-08-24', \
                 $7, $8, $9, $10, $11)",
    )
    .bind(mismatched_snapshot_run)
    .bind(owner_a)
    .bind(config_a)
    .bind(serde_json::json!({"benchmark":"069500.KRX"}))
    .bind(&strategy_hash)
    .bind(mismatched_snapshot_job)
    .bind(&candidate_hash)
    .bind(&artifact_hash)
    .bind(&stage5_hash)
    .bind(&action_hash)
    .bind(&approval_hash)
    .execute(&app_a)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&mismatched_snapshot).as_deref(), Some("23514"));
    let linked_job_payload_rewrite =
        sqlx::query("UPDATE jobs SET payload_json = '{}'::jsonb WHERE id = $1")
            .bind(job_a)
            .execute(&owner_a_actor)
            .await
            .unwrap_err();
    assert_eq!(
        pg_code(&linked_job_payload_rewrite).as_deref(),
        Some("42501")
    );
    let immutable_strategy_snapshot = sqlx::query(
        "UPDATE owner_beta_recommendation_runs SET strategy_version = '9.9.9' WHERE id = $1",
    )
    .bind(run_a)
    .execute(&owner_a_actor)
    .await
    .unwrap_err();
    assert_eq!(
        pg_code(&immutable_strategy_snapshot).as_deref(),
        Some("42501")
    );

    MIGRATOR.run_to(51, owner).await?;
    assert_eq!(applied_count(owner).await?, 51);
    MIGRATOR.run(owner).await?;
    assert_eq!(applied_count(owner).await?, up_migration_count() as i64);
    // 0053 has its own runtime/rollback contract below. Keep this 0049--0052
    // failure/reconnect scenario at its original migration frontier so the
    // expected failed 0051 down migration remains the first failed attempt and
    // SQLx cannot strand a lock after first successfully reverting 0053.
    MIGRATOR.undo(owner, 52).await?;
    assert_eq!(applied_count(owner).await?, 52);

    sqlx::query(
        "INSERT INTO instruments (id, symbol, venue, currency) \
         VALUES ('owner-beta-a.KRX', 'owner-beta-a', 'KRX', 'KRW'), \
                ('owner-beta-b.KRX', 'owner-beta-b', 'KRX', 'KRW')",
    )
    .execute(owner)
    .await?;
    let worker = role_pool(super_url, db, "worker").await?;
    for (run_id, owner_id, instrument_id) in [
        (run_a, owner_a, "owner-beta-a.KRX"),
        (run_b, owner_b, "owner-beta-b.KRX"),
    ] {
        sqlx::query(
            "INSERT INTO owner_beta_recommendation_items \
             (recommendation_run_id, owner_user_id, instrument_id, rank, target_weight) \
             VALUES ($1, $2, $3, 1, 1.000000)",
        )
        .bind(run_id)
        .bind(owner_id)
        .bind(instrument_id)
        .execute(&worker)
        .await?;
    }

    let app_a_visible: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM owner_beta_recommendation_runs), \
                (SELECT count(*) FROM owner_beta_recommendation_items)",
    )
    .fetch_one(&app_a)
    .await?;
    assert_eq!(app_a_visible, (1, 1), "owner A cannot read owner B rows");
    let owner_a_cross_mutation =
        sqlx::query("UPDATE owner_beta_recommendation_runs SET status = 'FAILED' WHERE id = $1")
            .bind(run_b)
            .execute(&owner_a_actor)
            .await?;
    assert_eq!(
        owner_a_cross_mutation.rows_affected(),
        0,
        "forced RLS prevents owner A from mutating owner B's run"
    );
    let unscoped_migration_owner_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM owner_beta_recommendation_runs")
            .fetch_one(owner)
            .await?;
    assert_eq!(
        unscoped_migration_owner_visible, 0,
        "FORCE RLS also applies to migration_owner without an actor GUC"
    );
    let admin = role_pool(super_url, db, "admin").await?;
    let admin_visible: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM owner_beta_recommendation_runs), \
                (SELECT count(*) FROM owner_beta_recommendation_items)",
    )
    .fetch_one(&admin)
    .await?;
    assert_eq!(admin_visible, (2, 2));

    for (role, run_privileges, item_privileges) in [
        (
            "app",
            (true, false, false, false),
            (true, false, false, false),
        ),
        (
            "worker",
            (true, false, false, false),
            (true, false, false, false),
        ),
        (
            "admin",
            (true, false, false, false),
            (true, false, false, false),
        ),
    ] {
        let actual: (bool, bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT \
               has_table_privilege($1, 'owner_beta_recommendation_runs', 'SELECT'), \
               has_table_privilege($1, 'owner_beta_recommendation_runs', 'INSERT'), \
               has_table_privilege($1, 'owner_beta_recommendation_runs', 'UPDATE'), \
               has_table_privilege($1, 'owner_beta_recommendation_runs', 'DELETE'), \
               has_table_privilege($1, 'owner_beta_recommendation_items', 'SELECT'), \
               has_table_privilege($1, 'owner_beta_recommendation_items', 'INSERT'), \
               has_table_privilege($1, 'owner_beta_recommendation_items', 'UPDATE'), \
               has_table_privilege($1, 'owner_beta_recommendation_items', 'DELETE')",
        )
        .bind(role)
        .fetch_one(owner)
        .await?;
        assert_eq!(
            actual,
            (
                run_privileges.0,
                run_privileges.1,
                run_privileges.2,
                run_privileges.3,
                item_privileges.0,
                item_privileges.1,
                item_privileges.2,
                item_privileges.3,
            ),
            "{role} grants drifted"
        );
    }
    for column in [
        "id",
        "owner_user_id",
        "strategy_config_id",
        "strategy_id",
        "strategy_version",
        "strategy_config_json",
        "strategy_config_sha256",
        "job_id",
        "as_of",
        "candidate_content_sha256",
        "artifact_manifest_sha256",
        "stage5_manifest_sha256",
        "action_manifest_sha256",
        "approval_registry_sha256",
    ] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege('app', 'owner_beta_recommendation_runs', $1, 'INSERT')",
        )
        .bind(column)
        .fetch_one(owner)
        .await?;
        assert!(allowed, "app INSERT grant missing for {column}");
    }
    let app_result_insert: bool = sqlx::query_scalar(
        "SELECT has_column_privilege(\
            'app', 'owner_beta_recommendation_runs', 'factor_snapshot_sha256', 'INSERT')",
    )
    .fetch_one(owner)
    .await?;
    assert!(!app_result_insert, "app must not insert result columns");
    for column in [
        "status",
        "factor_snapshot_sha256",
        "target_snapshot_sha256",
        "cash_weight",
        "error_code",
        "started_at",
        "finished_at",
        "updated_at",
    ] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege(\
                'worker', 'owner_beta_recommendation_runs', $1, 'UPDATE')",
        )
        .bind(column)
        .fetch_one(owner)
        .await?;
        assert!(allowed, "worker UPDATE grant missing for {column}");
    }
    let worker_lineage_update: bool = sqlx::query_scalar(
        "SELECT has_column_privilege(\
            'worker', 'owner_beta_recommendation_runs', 'strategy_version', 'UPDATE')",
    )
    .fetch_one(owner)
    .await?;
    assert!(
        !worker_lineage_update,
        "worker must not update lineage columns"
    );
    for column in [
        "id",
        "recommendation_run_id",
        "owner_user_id",
        "instrument_id",
        "rank",
        "target_weight",
        "reason_codes",
        "factors_json",
        "excluded",
        "exclusion_reason",
    ] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege(\
                'worker', 'owner_beta_recommendation_items', $1, 'INSERT')",
        )
        .bind(column)
        .fetch_one(owner)
        .await?;
        assert!(allowed, "worker item INSERT grant missing for {column}");
    }
    let app_item_insert = sqlx::query(
        "INSERT INTO owner_beta_recommendation_items \
         (recommendation_run_id, owner_user_id, instrument_id) \
         VALUES ($1, $2, 'owner-beta-a.KRX')",
    )
    .bind(run_a)
    .bind(owner_a)
    .execute(&app_a)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&app_item_insert).as_deref(), Some("42501"));
    let worker_item_rewrite = sqlx::query(
        "UPDATE owner_beta_recommendation_items SET rank = 2 WHERE recommendation_run_id = $1",
    )
    .bind(run_a)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&worker_item_rewrite).as_deref(), Some("42501"));
    let worker_item_delete =
        sqlx::query("DELETE FROM owner_beta_recommendation_items WHERE recommendation_run_id = $1")
            .bind(run_a)
            .execute(&worker)
            .await
            .unwrap_err();
    assert_eq!(pg_code(&worker_item_delete).as_deref(), Some("42501"));

    sqlx::query(
        "UPDATE owner_beta_recommendation_runs \
         SET status = 'SUCCEEDED', factor_snapshot_sha256 = $2, \
             target_snapshot_sha256 = $3, cash_weight = 0.000000 \
         WHERE id = $1",
    )
    .bind(run_a)
    .bind(&factor_hash)
    .bind(&target_hash)
    .execute(&worker)
    .await?;
    let rollback_guard = MIGRATOR.undo(owner, 50).await.unwrap_err();
    assert_eq!(migrate_pg_code(&rollback_guard).as_deref(), Some("55000"));
    assert_eq!(applied_count(owner).await?, 51);
    // sqlx uses a session-level advisory lock for migrations. An expected
    // migration failure can leave that lock on the pooled connection that ran
    // it, so a second `undo` through another connection in the same pool would
    // wait on itself. Close the failed-attempt pool and reconnect exactly as
    // migration_owner before exercising the successful rollback path.
    owner.close().await;
    let reconnected_owner = effective_role_pool(super_url, db, "migration_owner", None, 3).await?;
    let owner = &reconnected_owner;
    sqlx::query(
        "UPDATE owner_beta_recommendation_runs \
         SET status = 'PENDING', factor_snapshot_sha256 = NULL, \
             target_snapshot_sha256 = NULL, cash_weight = NULL \
         WHERE id = $1",
    )
    .bind(run_a)
    .execute(&worker)
    .await?;
    MIGRATOR.undo(owner, 50).await?;
    assert_eq!(applied_count(owner).await?, 50);
    MIGRATOR.run(owner).await?;
    assert_eq!(applied_count(owner).await?, up_migration_count() as i64);
    Ok(())
}

#[test]
fn candidate_vertical_contract_is_separate_pit_and_fail_closed() {
    for token in [
        "candidate_universe_snapshots",
        "candidate_universe_members",
        "candidate_investor_flows",
        "candidate_investor_flow_snapshot_rows",
        "candidate_market_status_observations",
        "candidate_fundamental_observations",
        "candidate_sector_versions",
        "candidate_sector_entries",
        "fundamental_profile",
        "available_at",
        "source_revision",
        "license_ref",
        "entitlement_id",
        "entitlement_date",
        "candidate_price_publications",
        "candidate_price_instrument_coverage",
        "candidate_price_instrument_sessions",
        "candidate_raw_batch_publications",
        "candidate_raw_batch_datasets",
        "candidate_raw_dataset_single_origin_idx",
        "begin_candidate_raw_batch",
        "bind_candidate_raw_dataset",
        "seal_candidate_raw_batch",
        "block_candidate_raw_batch_for_inactive_rights",
        "ENTITLEMENT_INACTIVE",
        "candidate_instrument_registrations",
        "curated_generation",
        "candidate_source_entitlement_is_valid",
        "resolve_candidate_contract_entitlement",
        "register_candidate_instrument",
        "register_candidate_source_dataset",
        "publish_candidate_price_publication",
        "p_instrument_coverage jsonb",
        "candidate price coverage conflicts with immutable generation",
        "v_expected_storage_path := 'db://candidate/'",
        "candidate source catalog requires exact active candidate-use rights",
        "covered_uses @> '[\"candidate\"]'::jsonb",
        "candidate_source_validate_dataset_pin",
        "SECURITY DEFINER",
        "candidate_universe_validate_members",
        "DEFERRABLE INITIALLY DEFERRED",
        "FORCE ROW LEVEL SECURITY",
        "GRANT SELECT ON TABLE public.dataset_versions TO research_writer",
        "candidate_dataset_versions_select_research_writer",
        "insert_candidate_investor_flow",
        "insert_candidate_market_status",
        "insert_candidate_fundamental",
        "pg_catalog.pg_advisory_xact_lock",
        "candidate universe-member natural identity is occupied by different content",
        "candidate fundamental restatement lineage is invalid",
        "p_entitlement_date <> p_last_session",
    ] {
        assert!(
            CANDIDATE_SOURCE_UP_SQL.contains(token),
            "0042 candidate source contract is missing {token}"
        );
    }
    assert!(
        !CANDIDATE_SOURCE_UP_SQL
            .contains("GRANT INSERT ON TABLE public.dataset_versions TO research_writer"),
        "research_writer must only catalog through narrow definer procedures"
    );
    assert!(
        !CANDIDATE_SOURCE_UP_SQL.contains("GRANT INSERT ON TABLE public.%I TO research_writer"),
        "research_writer must publish typed source rows only through narrow definers"
    );
    for token in [
        "published candidate source observations",
        "candidate_instrument_registrations",
        "candidate_price_instrument_coverage",
        "candidate_price_instrument_sessions",
        "candidate_investor_flow_snapshot_rows",
        "candidate_raw_batch_publications",
        "candidate_raw_batch_datasets",
        "DROP FUNCTION public.seal_candidate_raw_batch",
        "candidate_source_validate_dataset_pin",
        "candidate_universe_validate_members",
        "REVOKE SELECT ON TABLE public.dataset_versions FROM research_writer",
        "DROP POLICY candidate_dataset_versions_select_research_writer",
    ] {
        assert!(
            CANDIDATE_SOURCE_DOWN_SQL.contains(token),
            "0042 rollback is missing {token}"
        );
    }

    for token in [
        "candidate_scoring_configs",
        "stock_analysis_runs",
        "stock_analysis_snapshots",
        "candidate_feed_snapshots",
        "candidate_feed_items",
        "screener_saved_screens",
        "CREATE POLICY screener_saved_screens_owner",
        "candidate-score-v1",
        "1cd70f7a79af85896b015f265bea8ae931bbba29aef12a0b95f32c82ee056377",
        "min_average_trading_value_20",
        "price_curated_version",
        "price_entitlement_id",
        "universe_entitlement_id",
        "status_dataset_version_id",
        "status_entitlement_id",
        "status_manifest_sha256",
        "flow_entitlement_id",
        "fundamental_entitlement_id",
        "sector_entitlement_id",
        "UNIVERSE_FALLBACK",
        "STRONG', 'MODERATE', 'WEAK",
        "published candidate feed must contain exactly five items",
        "screener_saved_screens_app",
        "NULLIF(current_setting('app.actor_user_id', true), '')::uuid",
    ] {
        assert!(
            CANDIDATE_ANALYSIS_UP_SQL.contains(token),
            "0043 candidate analysis contract is missing {token}"
        );
    }
    assert!(
        !CANDIDATE_ANALYSIS_UP_SQL.contains("GRANT INSERT ON TABLE public.stock_analysis")
            && !CANDIDATE_ANALYSIS_UP_SQL.contains("GRANT UPDATE ON TABLE public.stock_analysis"),
        "serving roles must not receive candidate publication DML"
    );
    assert!(
        CANDIDATE_ANALYSIS_DOWN_SQL
            .contains("rollback blocked by candidate analysis or saved screens"),
        "0043 rollback must preserve published/user data"
    );

    for token in [
        "candidate-scheduler@system.invalid",
        "candidate_scheduler_control",
        "required_fetch_mode",
        "jobs_reject_candidate_scheduled_mutation",
        "candidate:scheduled:",
        "candidate_compute",
        "schedule_candidate_run",
        "candidate source pins are not sealed under the required fetch mode",
        "candidate cutoff does not match exact pinned source availability",
        "candidate run requires 60 confirmed KRX sessions",
        "v_required_first_session, p_as_of_date",
        "candidate_published_source_attributions",
        "refs.first_use_date, refs.last_use_date",
        "'app.actor_user_id', v_service_user_id::text, true",
        "publish_candidate_analysis",
        "fail_candidate_analysis_run",
        "assert_candidate_publication_settlement",
        "candidate publication and queue settlement must commit atomically",
        "p_summary IS DISTINCT FROM v_run.summary_json",
        "jsonb_path_query",
        "content_sha256",
        "count(DISTINCT supplied.value ->> 'instrument_id')",
        "GRANT EXECUTE ON FUNCTION public.schedule_candidate_run",
        "GRANT EXECUTE ON FUNCTION public.publish_candidate_analysis",
        "TO worker",
    ] {
        assert!(
            CANDIDATE_PIPELINE_UP_SQL.contains(token),
            "0044 candidate pipeline contract is missing {token}"
        );
    }
    assert!(
        !CANDIDATE_PIPELINE_UP_SQL.contains("INSERT INTO public.recommendation_runs")
            && !CANDIDATE_PIPELINE_UP_SQL.contains("INSERT INTO public.recommendation_items")
            && !CANDIDATE_PIPELINE_UP_SQL.contains("INSERT INTO public.target_portfolios")
            && !CANDIDATE_PIPELINE_UP_SQL.contains("recommendation:scheduled:"),
        "candidate publication must remain separate from ETF recommendations"
    );
    assert_eq!(
        CANDIDATE_PIPELINE_UP_SQL
            .matches("entitlement became inactive before publication")
            .count(),
        6,
        "publication must re-attest all six exact entitlements"
    );
    assert!(
        CANDIDATE_PIPELINE_UP_SQL.contains("supplied.value ? 'content_sha256'")
            && CANDIDATE_PIPELINE_UP_SQL
                .contains("pg_catalog.sha256(pg_catalog.jsonb_send(value))"),
        "PostgreSQL must own snapshot hashes and reject worker-supplied hashes"
    );
    assert!(
        CANDIDATE_SCHEDULE_RS.contains("canonical_cutoff")
            && CANDIDATE_SCHEDULE_RS.contains("candidate_price_publications")
            && CANDIDATE_SCHEDULE_RS.contains("candidate_price_instrument_sessions")
            && CANDIDATE_SCHEDULE_RS.contains("MIN_PRICE_CONTEXT_SESSIONS")
            && CANDIDATE_SCHEDULE_RS.contains("required_sessions AS MATERIALIZED")
            && CANDIDATE_SCHEDULE_RS.contains("price_session.session_date=required.session_date")
            && !CANDIDATE_SCHEDULE_RS.contains("CANDIDATE_PRICE_CURATED_VERSION"),
        "scheduler identity must be derived from exact publications, not poll time or env"
    );
    assert!(
        CANDIDATE_RUNNER_RS.contains("read_dataset_manifest")
            && CANDIDATE_RUNNER_RS.contains("dataset_manifest_hash"),
        "candidate computation must attest the on-disk price manifest"
    );
    assert!(
        CANDIDATE_PIPELINE_UP_SQL.contains("candidate_price_instrument_sessions")
            && CANDIDATE_PIPELINE_UP_SQL.contains("LIMIT 60")
            && CANDIDATE_PIPELINE_UP_SQL
                .contains("price_session.session_date=required.session_date")
            && CANDIDATE_PIPELINE_UP_SQL
                .contains("fewer than five candidate members have complete 60-session inputs"),
        "direct candidate scheduling must require five viable 60-session members"
    );
    assert!(
        RESEARCH_WORKER_RS.contains("run_candidate_source_ingest")
            && RESEARCH_WORKER_RS.contains("RESEARCH_CANDIDATE_ENABLED")
            && RESEARCH_WORKER_RS.contains("batch.fetch_mode = $3")
            && RESEARCH_WORKER_RS.contains("trade_date = $1 AND entitlement_date = $1")
            && RESEARCH_WORKER_RS.contains("fiscal_period_end <= $1")
            && RESEARCH_WORKER_RS.contains("effective_from <= $1")
            && RESEARCH_WORKER_RS.contains("FROM trading_calendars AS calendar")
            && RESEARCH_WORKER_RS.contains("calendar.session_type = 'TRADING'")
            && RESEARCH_WORKER_RS.contains("candidate_price_instrument_sessions")
            && RESEARCH_WORKER_RS
                .contains("price.first_session <= $1 AND price.last_session >= $1"),
        "candidate Raw-to-publication path must be wired into the production worker"
    );
    for token in [
        "pg_advisory_xact_lock(1815099521, 44)",
        "rollback blocked by candidate job or run lineage",
        "reserved service principal dependencies",
        "FROM public.screener_saved_screens",
        "DROP FUNCTION public.assert_candidate_publication_settlement",
    ] {
        assert!(
            CANDIDATE_PIPELINE_DOWN_SQL.contains(token),
            "0044 rollback is missing {token}"
        );
    }

    for sql in [
        CANDIDATE_SOURCE_UP_SQL,
        CANDIDATE_ANALYSIS_UP_SQL,
        CANDIDATE_PIPELINE_UP_SQL,
    ] {
        assert!(!sql.contains("pg_catalog.nullif"));
        assert!(!sql.contains("pg_catalog.coalesce"));
        assert!(!sql.contains("pg_catalog.extract"));
    }
}

/// Live boundary proof for the three candidate migrations. CI supplies the
/// disposable PostgreSQL supervisor; developer machines without one retain
/// the static contract above and skip this probe explicitly.
#[tokio::test]
async fn candidate_vertical_roles_rls_and_rollback_are_fail_closed() {
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

        let price_attestation_privileges: (bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT
                has_function_privilege('worker',
                  'public.price_dataset_entitlement_is_valid(uuid,text,date,date)', 'EXECUTE'),
                has_function_privilege('research_writer',
                  'public.price_dataset_entitlement_is_valid(uuid,text,date,date)', 'EXECUTE'),
                has_function_privilege('app',
                  'public.price_dataset_entitlement_is_valid(uuid,text,date,date)', 'EXECUTE'),
                has_function_privilege('admin',
                  'public.price_dataset_entitlement_is_valid(uuid,text,date,date)', 'EXECUTE'),
                has_function_privilege('audit_writer',
                  'public.price_dataset_entitlement_is_valid(uuid,text,date,date)', 'EXECUTE')",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(
            price_attestation_privileges,
            (true, true, false, false, false),
            "price entitlement attestation must be limited to worker and research_writer"
        );

        let privileges: (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT
                has_table_privilege('research_writer', 'public.dataset_versions', 'SELECT'),
                has_table_privilege('research_writer', 'public.candidate_investor_flows', 'INSERT'),
                has_table_privilege('research_writer', 'public.candidate_investor_flows', 'UPDATE'),
                has_table_privilege('app', 'public.stock_analysis_runs', 'INSERT'),
                has_function_privilege('worker',
                  'public.publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)', 'EXECUTE'),
                has_function_privilege('app',
                  'public.publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)', 'EXECUTE')",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(privileges, (true, false, false, false, true, false));

        let app = role_pool(&super_url, &db, "app").await?;
        let worker = role_pool(&super_url, &db, "worker").await?;
        let research_writer = role_pool(&super_url, &db, "research_writer").await?;
        let app_insert = sqlx::query("INSERT INTO stock_analysis_runs DEFAULT VALUES")
            .execute(&app)
            .await
            .unwrap_err();
        assert_eq!(pg_code(&app_insert).as_deref(), Some("42501"));
        let worker_insert = sqlx::query("INSERT INTO candidate_feed_snapshots DEFAULT VALUES")
            .execute(&worker)
            .await
            .unwrap_err();
        assert_eq!(pg_code(&worker_insert).as_deref(), Some("42501"));
        let source_update = sqlx::query("UPDATE candidate_investor_flows SET net_amount = 0")
            .execute(&research_writer)
            .await
            .unwrap_err();
        assert_eq!(pg_code(&source_update).as_deref(), Some("42501"));
        drop(app);
        drop(worker);
        drop(research_writer);

        let service_user = "00000000-0000-4000-8000-000000000099";
        let other_user = "00000000-0000-4000-8000-000000000043";
        sqlx::query(
            "INSERT INTO users (id,issuer,subject,email,display_name)
             VALUES ($1,'urn:lagrange:test','candidate-rollback-owner',
                     'candidate-rollback-owner@example.invalid','Candidate rollback owner')",
        )
        .bind(Uuid::parse_str(service_user)?)
        .execute(&owner)
        .await?;
        let service_actor = actor_pool(&super_url, &db, "app", service_user).await?;
        let screen_id: Uuid = sqlx::query_scalar(
            "INSERT INTO screener_saved_screens
                (owner_user_id, name, criteria_schema_version, criteria_json)
             VALUES ($1, 'rollback-boundary', 1, '{}'::jsonb)
             RETURNING id",
        )
        .bind(Uuid::parse_str(service_user)?)
        .fetch_one(&service_actor)
        .await?;
        let other_actor = actor_pool(&super_url, &db, "app", other_user).await?;
        let leaked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM screener_saved_screens WHERE id = $1",
        )
        .bind(screen_id)
        .fetch_one(&other_actor)
        .await?;
        assert_eq!(leaked, 0, "saved screens must remain actor-private");
        drop(other_actor);

        MIGRATOR.undo(&owner, 43).await?;
        // A failed SQLx migration retains its session advisory lock. Keep the
        // guarded rollback on one acquired connection and release that lock
        // explicitly before the successful retry below.
        let mut guarded = owner.acquire().await?;
        let blocked = MIGRATOR.undo(&mut *guarded, 42).await.unwrap_err();
        sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *guarded)
            .await?;
        drop(guarded);
        assert_eq!(migrate_pg_code(&blocked).as_deref(), Some("55000"));
        let retained: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM screener_saved_screens WHERE id = $1)",
        )
        .bind(screen_id)
        .fetch_one(&service_actor)
        .await?;
        assert!(retained, "blocked rollback must not cascade-delete a saved screen");
        sqlx::query("DELETE FROM screener_saved_screens WHERE id = $1")
            .bind(screen_id)
            .execute(&service_actor)
            .await?;
        drop(service_actor);

        MIGRATOR.undo(&owner, 41).await?;
        let removed: (bool, bool, bool) = sqlx::query_as(
            "SELECT
                to_regclass('public.candidate_universe_snapshots') IS NULL,
                to_regclass('public.stock_analysis_runs') IS NULL,
                to_regclass('public.candidate_scheduler_control') IS NULL",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(removed, (true, true, true));
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("candidate migration boundaries FAILED: {error}");
    }
}

#[test]
fn candidate_multi_universe_contract_is_registry_and_identity_scoped() {
    for token in [
        "candidate_universe_registry",
        "kospi200",
        "kosdaq150",
        "krx_kospi200_membership",
        "krx_kosdaq150_membership",
        "FORCE ROW LEVEL SECURITY",
        "candidate_raw_batch_datasets",
        "dataset_id",
        "candidate_raw_dataset_id_matches",
        "UNIQUE (universe_key, as_of_date, computation_seq)",
        "candidate_feed_active_date_uq",
        "candidate_feed_latest_idx",
        "candidate source batch is incomplete and cannot be sealed",
        "candidate source pins are not sealed under the required fetch mode",
        "candidate publication replay payload mismatch",
        "previous.universe_key = v_run.universe_key",
        "candidate|' || v_universe_key || '|' || p_as_of_date::text",
        "candidate_published_source_attributions",
        "feed.status IN ('PUBLISHED','SUPERSEDED')",
    ] {
        assert!(
            CANDIDATE_MULTI_UNIVERSE_UP_SQL.contains(token),
            "0045 up is missing multi-universe contract token {token}"
        );
    }
    for token in [
        "0045 rollback blocked by KOSDAQ candidate identity or history",
        "candidate_0045_scheduler_state",
        "SELECT min(registry.created_at)",
        "CREATE OR REPLACE FUNCTION public.schedule_candidate_run",
        "CREATE OR REPLACE FUNCTION public.publish_candidate_analysis",
        "CREATE OR REPLACE FUNCTION public.candidate_published_source_attributions",
        "original PUBLISHED-only serving contract",
        "candidate_raw_batch_datasets_pkey",
        "DROP TABLE public.candidate_universe_registry",
    ] {
        assert!(
            CANDIDATE_MULTI_UNIVERSE_DOWN_SQL.contains(token),
            "0045 down is missing guarded rollback token {token}"
        );
    }
    assert!(
        !CANDIDATE_MULTI_UNIVERSE_UP_SQL.contains("candidate_0045_function_backup")
            && !CANDIDATE_MULTI_UNIVERSE_DOWN_SQL.contains("candidate_0045_function_backup"),
        "0045 rollback must be self-contained and must not depend on a persistent backup table"
    );
}

#[test]
fn candidate_price_revalidation_is_exact_and_append_only() {
    for token in [
        "CREATE FUNCTION public.price_dataset_entitlement_is_valid",
        "[\"dataset\",\"recommendation\",\"backtest\",\"paper_view\"]",
        "CREATE FUNCTION public.resolve_price_dataset_entitlement",
        "CREATE TABLE public.candidate_price_revalidation_events",
        "blocked_first_date",
        "revalidated_first_date",
        "rights_first_date",
        "rights_last_date",
        "CREATE FUNCTION public.revalidate_candidate_price_raw_batch",
        "ENTITLEMENT_REVALIDATED",
        "FORCE ROW LEVEL SECURITY",
        "candidate_price_revalidation_events_immutable",
    ] {
        assert!(
            CANDIDATE_PRICE_REVALIDATION_UP_SQL.contains(token),
            "0046 up is missing price revalidation token {token}"
        );
    }
    assert!(!CANDIDATE_PRICE_REVALIDATION_UP_SQL.contains("candidate_price_revalidation_exact_uq"));
    assert!(
        CANDIDATE_PRICE_REVALIDATION_UP_SQL
            .contains("price requires an exact active price dataset entitlement")
    );
    assert!(
        CANDIDATE_PRICE_REVALIDATION_UP_SQL.contains("covered_uses @> '[\"candidate\"]'::jsonb")
    );
    assert!(CANDIDATE_PRICE_REVALIDATION_UP_SQL.contains("state = 'CATALOGED'"));
    assert!(CANDIDATE_PRICE_REVALIDATION_UP_SQL.contains("ON DELETE RESTRICT"));
    assert!(
        CANDIDATE_PRICE_REVALIDATION_DOWN_SQL
            .contains("0046 rollback blocked by price revalidation history")
    );
    assert!(CANDIDATE_PRICE_REVALIDATION_DOWN_SQL.contains("ERRCODE = '55000'"));
    assert!(
        CANDIDATE_PRICE_REVALIDATION_DOWN_SQL
            .contains("DROP TABLE public.candidate_price_revalidation_events")
    );
    assert!(
        CANDIDATE_PRICE_REVALIDATION_DOWN_SQL.contains("covered_uses @> '[\"candidate\"]'::jsonb")
    );
}

#[test]
fn candidate_worker_price_attestation_grant_is_narrow_and_reversible() {
    for token in [
        "GRANT EXECUTE ON FUNCTION",
        "public.price_dataset_entitlement_is_valid(uuid, text, date, date)",
        "TO worker",
    ] {
        assert!(
            CANDIDATE_WORKER_PRICE_ATTESTATION_UP_SQL.contains(token),
            "0047 up is missing worker price-attestation token {token}"
        );
    }
    assert!(!CANDIDATE_WORKER_PRICE_ATTESTATION_UP_SQL.contains("TO PUBLIC"));
    assert!(!CANDIDATE_WORKER_PRICE_ATTESTATION_UP_SQL.contains("ON TABLE"));
    assert!(CANDIDATE_WORKER_PRICE_ATTESTATION_DOWN_SQL.contains("REVOKE EXECUTE ON FUNCTION"));
    assert!(
        CANDIDATE_WORKER_PRICE_ATTESTATION_DOWN_SQL
            .contains("public.price_dataset_entitlement_is_valid(uuid, text, date, date)")
    );
    assert!(CANDIDATE_WORKER_PRICE_ATTESTATION_DOWN_SQL.contains("FROM worker"));
}

async fn reopen_price_raw_batch(
    pool: &PgPool,
    batch_id: Uuid,
    raw_hash: &str,
    contract_reference: &str,
    entitlement_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT public.revalidate_candidate_price_raw_batch(\
            $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date,\
            '2020-01-01'::date, '2026-08-18'::date, $4)",
    )
    .bind(batch_id)
    .bind(raw_hash)
    .bind(contract_reference)
    .bind(entitlement_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Live PostgreSQL proof for the rights-reopen convergence boundary.  The
/// fixture mirrors the production failure shape: the newest price delivery
/// was blocked on its single source day while an older delivery is still
/// CATALOGED.  Revalidation widens the requested window, but the audit keeps
/// the original blocked window so a replay cannot silently change identity.
#[tokio::test]
async fn candidate_price_rights_reopen_is_audited_and_idempotent() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };

    let result: Result<(), Box<dyn Error>> = async {
        MIGRATOR.run(&owner).await?;

        let managed_by = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, issuer, subject, email) \
             VALUES ($1, 'rights-reopen-test', $2, $3)",
        )
        .bind(managed_by)
        .bind(managed_by.to_string())
        .bind(format!(
            "rights-reopen-{}@example.test",
            managed_by.simple()
        ))
        .execute(&owner)
        .await?;

        let entitlement_id = Uuid::new_v4();
        let contract_reference = "rights-reopen-contract";
        sqlx::query(
            "INSERT INTO data_entitlements \
             (id, contract_document_sha256, contract_reference, status,\
              covered_datasets, covered_uses, effective_from, effective_until, managed_by) \
             VALUES ($1, $2, $3, 'PENDING',\
                     '[\"krx_eod_bars\"]'::jsonb,\
                     '[\"dataset\",\"recommendation\",\"backtest\",\"paper_view\"]'::jsonb,\
                     '2020-01-01'::date, '2026-08-18'::date, $4)",
        )
        .bind(entitlement_id)
        .bind("a".repeat(64))
        .bind(contract_reference)
        .bind(managed_by)
        .execute(&owner)
        .await?;

        let blocked_batch = Uuid::new_v4();
        let blocked_hash = "b".repeat(64);
        sqlx::query(
            "SELECT public.begin_candidate_raw_batch(\
                $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date)",
        )
        .bind(blocked_batch)
        .bind(&blocked_hash)
        .bind(contract_reference)
        .execute(&owner)
        .await?;
        sqlx::query(
            "SELECT public.block_candidate_raw_batch_for_inactive_rights(\
                $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date,\
                '2026-08-18'::date, '2026-08-18'::date)",
        )
        .bind(blocked_batch)
        .bind(&blocked_hash)
        .bind(contract_reference)
        .execute(&owner)
        .await?;

        // This older source is the pending cumulative suffix seen by worker
        // recovery when the latest day is already terminal.
        let older_batch = Uuid::new_v4();
        let older_hash = "c".repeat(64);
        sqlx::query(
            "SELECT public.begin_candidate_raw_batch(\
                $1, 'price', $2, 'credentialed', $3, '2020-01-01'::date)",
        )
        .bind(older_batch)
        .bind(&older_hash)
        .bind(contract_reference)
        .execute(&owner)
        .await?;

        let inactive_resolve = sqlx::query_scalar::<_, Uuid>(
            "SELECT public.resolve_price_dataset_entitlement(\
                $1, '2020-01-01'::date, '2026-08-18'::date)",
        )
        .bind(contract_reference)
        .fetch_one(&owner)
        .await
        .expect_err("PENDING rights must not resolve");
        assert_eq!(pg_code(&inactive_resolve).as_deref(), Some("42501"));

        let inactive_revalidate = sqlx::query(
            "SELECT public.revalidate_candidate_price_raw_batch(\
                $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date,\
                '2020-01-01'::date, '2026-08-18'::date, $4)",
        )
        .bind(blocked_batch)
        .bind(&blocked_hash)
        .bind(contract_reference)
        .bind(entitlement_id)
        .execute(&owner)
        .await
        .expect_err("inactive rights must not reopen a blocked Raw row");
        assert_eq!(pg_code(&inactive_revalidate).as_deref(), Some("42501"));

        sqlx::query("UPDATE data_entitlements SET status = 'ACTIVE' WHERE id = $1")
            .bind(entitlement_id)
            .execute(&owner)
            .await?;
        let resolved = sqlx::query_scalar::<_, Uuid>(
            "SELECT public.resolve_price_dataset_entitlement(\
                $1, '2020-01-01'::date, '2026-08-18'::date)",
        )
        .bind(contract_reference)
        .fetch_one(&owner)
        .await?;
        assert_eq!(resolved, entitlement_id);

        reopen_price_raw_batch(
            &owner,
            blocked_batch,
            &blocked_hash,
            contract_reference,
            entitlement_id,
        )
        .await?;

        let (event_count, blocked_day, revalidated_range): (i64, bool, bool) = sqlx::query_as(
            "SELECT count(*)::bigint,\
                    bool_and(blocked_first_date = '2026-08-18'::date \
                             AND blocked_last_date = '2026-08-18'::date),\
                    bool_and(revalidated_first_date = '2020-01-01'::date \
                             AND revalidated_last_date = '2026-08-18'::date) \
               FROM candidate_price_revalidation_events \
              WHERE batch_id = $1",
        )
        .bind(blocked_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(event_count, 1);
        assert!(blocked_day && revalidated_range);
        let state: String = sqlx::query_scalar(
            "SELECT state FROM candidate_raw_batch_publications \
              WHERE batch_id = $1 AND surface = 'price'",
        )
        .bind(blocked_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(state, "CATALOGED");

        // Crash/retry replay is a no-op, while a changed immutable hash is a
        // conflict even though the row is now CATALOGED.
        reopen_price_raw_batch(
            &owner,
            blocked_batch,
            &blocked_hash,
            contract_reference,
            entitlement_id,
        )
        .await?;
        let replay_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM candidate_price_revalidation_events \
              WHERE batch_id = $1",
        )
        .bind(blocked_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(replay_count, 1);
        let conflict = sqlx::query(
            "SELECT public.revalidate_candidate_price_raw_batch(\
                $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date,\
                '2020-01-01'::date, '2026-08-18'::date, $4)",
        )
        .bind(blocked_batch)
        .bind("d".repeat(64))
        .bind(contract_reference)
        .bind(entitlement_id)
        .execute(&owner)
        .await
        .expect_err("changed Raw hash must fail closed");
        assert_eq!(pg_code(&conflict).as_deref(), Some("23514"));

        // A later revocation/re-block/reactivation is a new real transition,
        // not a duplicate replay, and therefore appends a second event.
        sqlx::query("UPDATE data_entitlements SET status = 'REVOKED' WHERE id = $1")
            .bind(entitlement_id)
            .execute(&owner)
            .await?;
        sqlx::query(
            "SELECT public.block_candidate_raw_batch_for_inactive_rights(\
                $1, 'price', $2, 'credentialed', $3, '2026-08-18'::date,\
                '2026-08-18'::date, '2026-08-18'::date)",
        )
        .bind(blocked_batch)
        .bind(&blocked_hash)
        .bind(contract_reference)
        .execute(&owner)
        .await?;
        sqlx::query("UPDATE data_entitlements SET status = 'ACTIVE' WHERE id = $1")
            .bind(entitlement_id)
            .execute(&owner)
            .await?;
        reopen_price_raw_batch(
            &owner,
            blocked_batch,
            &blocked_hash,
            contract_reference,
            entitlement_id,
        )
        .await?;
        let second_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM candidate_price_revalidation_events \
              WHERE batch_id = $1",
        )
        .bind(blocked_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(second_count, 2);

        let older_state: String = sqlx::query_scalar(
            "SELECT state FROM candidate_raw_batch_publications \
              WHERE batch_id = $1 AND surface = 'price'",
        )
        .bind(older_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(older_state, "CATALOGED");

        let candidate_rights: bool = sqlx::query_scalar(
            "SELECT public.candidate_source_entitlement_is_valid(\
                $1, $2, 'krx_eod_bars', '2020-01-01'::date, '2026-08-18'::date)",
        )
        .bind(entitlement_id)
        .bind(contract_reference)
        .fetch_one(&owner)
        .await?;
        assert!(
            !candidate_rights,
            "price-only rights must not open candidate flows"
        );

        // Existing audit evidence makes rollback intentionally irreversible.
        let rollback = sqlx::raw_sql(CANDIDATE_PRICE_REVALIDATION_DOWN_SQL)
            .execute(&owner)
            .await
            .expect_err("0046 down must refuse to erase revalidation history");
        assert_eq!(pg_code(&rollback).as_deref(), Some("55000"));
        let audit_table: Option<String> = sqlx::query_scalar(
            "SELECT to_regclass('public.candidate_price_revalidation_events')::text",
        )
        .fetch_one(&owner)
        .await?;
        assert!(audit_table.is_some());
        Ok(())
    }
    .await;

    drop(owner);
    if let Err(error) = drop_contract_db(&super_url, &db).await {
        panic!("cleanup failed: {error}");
    }
    if let Err(error) = result {
        panic!("rights reopen contract failed: {error}");
    }
}

/// Live PostgreSQL 18 proof for the new registry, denormalized Raw binding
/// identity, guarded KOSDAQ rollback, and clean down/up replay.  The fixture
/// is intentionally tiny: it exercises the database boundary without
/// manufacturing a full 60-session candidate publication.
#[tokio::test]
async fn candidate_multi_universe_registry_and_guard_are_fail_closed() {
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
        let registry: Vec<(String, String, String, i32, bool)> = sqlx::query_as(
            "SELECT universe_key, membership_dataset_id, display_name, sort_order, enabled
               FROM public.candidate_universe_registry
              ORDER BY sort_order",
        )
        .fetch_all(&owner)
        .await?;
        assert_eq!(
            registry,
            vec![
                (
                    "kospi200".to_owned(),
                    "krx_kospi200_membership".to_owned(),
                    "KOSPI 200".to_owned(),
                    10,
                    true,
                ),
                (
                    "kosdaq150".to_owned(),
                    "krx_kosdaq150_membership".to_owned(),
                    "KOSDAQ 150".to_owned(),
                    20,
                    true,
                ),
            ]
        );
        let registry_rls: (bool, bool) = sqlx::query_as(
            "SELECT relrowsecurity, relforcerowsecurity
               FROM pg_class
              WHERE oid = 'public.candidate_universe_registry'::regclass",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(registry_rls, (true, true));
        let registry_policy_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM pg_policies
              WHERE schemaname = 'public'
                AND tablename = 'candidate_universe_registry'",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(registry_policy_count, 5);

        let binding_columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name
               FROM information_schema.columns
              WHERE table_schema = 'public'
                AND table_name = 'candidate_raw_batch_datasets'
              ORDER BY ordinal_position",
        )
        .fetch_all(&owner)
        .await?;
        assert!(binding_columns.iter().any(|column| column == "dataset_id"));
        let raw_pk: String = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(oid)
               FROM pg_constraint
              WHERE conrelid = 'public.candidate_raw_batch_datasets'::regclass
                AND contype = 'p'",
        )
        .fetch_one(&owner)
        .await?;
        assert!(raw_pk.contains("batch_id") && raw_pk.contains("dataset_id"));

        let run_date_key: String = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(oid)
               FROM pg_constraint
              WHERE conrelid = 'public.stock_analysis_runs'::regclass
                AND conname = 'stock_analysis_run_date_seq_key'",
        )
        .fetch_one(&owner)
        .await?;
        let feed_date_key: String = sqlx::query_scalar(
            "SELECT pg_get_constraintdef(oid)
               FROM pg_constraint
              WHERE conrelid = 'public.candidate_feed_snapshots'::regclass
                AND conname = 'candidate_feed_date_seq_key'",
        )
        .fetch_one(&owner)
        .await?;
        assert!(run_date_key.contains("universe_key") && run_date_key.contains("as_of_date"));
        assert!(feed_date_key.contains("universe_key") && feed_date_key.contains("as_of_date"));
        let latest_run_index: String = sqlx::query_scalar(
            "SELECT pg_get_indexdef(indexrelid)
               FROM pg_index
              WHERE indexrelid = 'public.stock_analysis_runs_latest_idx'::regclass",
        )
        .fetch_one(&owner)
        .await?;
        let active_feed_index: String = sqlx::query_scalar(
            "SELECT pg_get_indexdef(indexrelid)
               FROM pg_index
              WHERE indexrelid = 'public.candidate_feed_active_date_uq'::regclass",
        )
        .fetch_one(&owner)
        .await?;
        assert!(latest_run_index.contains("universe_key"));
        assert!(active_feed_index.contains("universe_key"));

        let grants: (bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT
                has_table_privilege('app', 'public.candidate_universe_registry', 'SELECT'),
                has_table_privilege('app', 'public.candidate_universe_registry', 'INSERT'),
                has_table_privilege('worker', 'public.candidate_universe_registry', 'UPDATE'),
                has_table_privilege('research_writer', 'public.candidate_universe_registry', 'DELETE'),
                has_function_privilege('worker',
                  'public.schedule_candidate_run(date,timestamptz,text,text,uuid,uuid,integer,text,uuid,text,uuid,text,uuid,text,uuid)',
                  'EXECUTE')",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(grants, (true, false, false, false, true));

        let app = role_pool(&super_url, &db, "app").await?;
        let direct_registry_insert = sqlx::query(
            "INSERT INTO public.candidate_universe_registry
                (universe_key, membership_dataset_id, display_name, market, sort_order, enabled)
             VALUES ('squat', 'krx_squat_membership', 'Squat', 'kr', 99, true)",
        )
        .execute(&app)
        .await
        .unwrap_err();
        assert_eq!(pg_code(&direct_registry_insert).as_deref(), Some("42501"));
        drop(app);

        let service_user_id: Uuid = sqlx::query_scalar(
            "SELECT service_user_id FROM public.candidate_scheduler_control
              WHERE control_key = 'scheduler'",
        )
        .fetch_one(&owner)
        .await?;
        let entitlement_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.data_entitlements
                (contract_document_sha256, contract_reference, status,
                 covered_datasets, covered_uses, effective_from, effective_until, managed_by)
             VALUES ($1, 'candidate-0045-test', 'ACTIVE',
                     '[\"krx_kosdaq150_membership\"]'::jsonb,
                     '[\"candidate\"]'::jsonb,
                     DATE '2026-01-01', DATE '2026-12-31', $2)
             RETURNING id",
        )
        .bind("a".repeat(64))
        .bind(service_user_id)
        .fetch_one(&owner)
        .await?;
        let dataset_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.dataset_versions
                (dataset_id, version, status, manifest_sha256, storage_path)
             VALUES ('krx_kosdaq150_membership', 'contract-test', 'READY', $1,
                     'db://candidate/krx_kosdaq150_membership/contract-test')
             RETURNING id",
        )
        .bind("b".repeat(64))
        .fetch_one(&owner)
        .await?;
        let instrument_id = format!("900{:0>8}.KRX", 45);
        sqlx::query(
            "INSERT INTO public.instruments
                (id, symbol, venue, currency, name, asset_class, status, listed_at)
             VALUES ($1, $2, 'KRX', 'KRW', '0045 contract', 'EQUITY', 'ACTIVE', DATE '2020-01-01')",
        )
        .bind(&instrument_id)
        .bind(instrument_id.trim_end_matches(".KRX"))
        .execute(&owner)
        .await?;
        let mut universe_tx = owner.begin().await?;
        let snapshot_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.candidate_universe_snapshots
                (index_id, as_of_date, dataset_version_id, manifest_sha256, provider,
                 entitlement_id, entitlement_date, license_ref, source_revision,
                 available_at, retrieved_at, member_count)
             VALUES ('kosdaq150', DATE '2026-08-14', $1, $2, 'krx', $3, DATE '2026-08-14',
                     'candidate-0045-test', 'contract-test',
                     TIMESTAMPTZ '2026-08-14 08:00:00+00',
                     TIMESTAMPTZ '2026-08-14 08:01:00+00', 1)
             RETURNING id",
        )
        .bind(dataset_id)
        .bind("b".repeat(64))
        .bind(entitlement_id)
        .fetch_one(&mut *universe_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_universe_members
                (universe_snapshot_id, instrument_id, announced_at, effective_from,
                 effective_until, available_at, source_revision)
             VALUES ($1, $2, TIMESTAMPTZ '2026-08-01 08:00:00+00', DATE '2026-08-14',
                     NULL, TIMESTAMPTZ '2026-08-14 08:00:00+00', 'contract-test')",
        )
        .bind(snapshot_id)
        .bind(&instrument_id)
        .execute(&mut *universe_tx)
        .await?;
        universe_tx.commit().await?;

        let mut guarded = owner.acquire().await?;
        let blocked = MIGRATOR.undo(&mut *guarded, 44).await.unwrap_err();
        sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *guarded)
            .await?;
        drop(guarded);
        assert_eq!(migrate_pg_code(&blocked).as_deref(), Some("55000"));
        let scheduler_after_guard: bool = sqlx::query_scalar(
            "SELECT active FROM public.candidate_scheduler_control
              WHERE control_key = 'scheduler'",
        )
        .fetch_one(&owner)
        .await?;
        assert!(scheduler_after_guard);
        let retained: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.candidate_universe_snapshots
              WHERE id = $1 AND index_id = 'kosdaq150'",
        )
        .bind(snapshot_id)
        .fetch_one(&owner)
        .await?;
        assert_eq!(retained, 1);

        sqlx::query("DELETE FROM public.candidate_universe_snapshots WHERE id = $1")
            .bind(snapshot_id)
            .execute(&owner)
            .await?;
        sqlx::query("DELETE FROM public.instruments WHERE id = $1")
            .bind(&instrument_id)
            .execute(&owner)
            .await?;
        sqlx::query("DELETE FROM public.dataset_versions WHERE id = $1")
            .bind(dataset_id)
            .execute(&owner)
            .await?;
        sqlx::query("DELETE FROM public.data_entitlements WHERE id = $1")
            .bind(entitlement_id)
            .execute(&owner)
            .await?;

        // A source publication sealed under 0045 proves that both enabled
        // universe memberships passed the completeness gate.  Simulate a
        // privileged repair that removed every binding row and ensure that
        // durable publication history still blocks the lossy rollback.
        let publication_only_batch = Uuid::parse_str("00000000-0000-4000-8000-000000000045")?;
        sqlx::query(
            "INSERT INTO public.candidate_raw_batch_publications
                (batch_id, surface, raw_manifest_sha256, fetch_mode,
                 entitlement_reference, entitlement_date, state, published_at)
             VALUES ($1, 'source', $2, 'synthetic', 'candidate-0045-test',
                     DATE '2026-08-14', 'PUBLISHED', clock_timestamp())",
        )
        .bind(publication_only_batch)
        .bind("c".repeat(64))
        .execute(&owner)
        .await?;

        let mut publication_guarded = owner.acquire().await?;
        let publication_blocked = MIGRATOR
            .undo(&mut *publication_guarded, 44)
            .await
            .unwrap_err();
        sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *publication_guarded)
            .await?;
        drop(publication_guarded);
        assert_eq!(
            migrate_pg_code(&publication_blocked).as_deref(),
            Some("55000")
        );
        let retained_publication: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.candidate_raw_batch_publications
              WHERE batch_id = $1 AND surface = 'source' AND state = 'PUBLISHED'",
        )
        .bind(publication_only_batch)
        .fetch_one(&owner)
        .await?;
        assert_eq!(retained_publication, 1);
        sqlx::query(
            "DELETE FROM public.candidate_raw_batch_publications
              WHERE batch_id = $1 AND surface = 'source'",
        )
        .bind(publication_only_batch)
        .execute(&owner)
        .await?;

        sqlx::query(
            "UPDATE public.candidate_scheduler_control
                SET active = false, updated_at = clock_timestamp()
              WHERE control_key = 'scheduler'",
        )
        .execute(&owner)
        .await?;
        MIGRATOR.undo(&owner, 44).await?;
        let removed: bool = sqlx::query_scalar(
            "SELECT to_regclass('public.candidate_universe_registry') IS NULL",
        )
        .fetch_one(&owner)
        .await?;
        assert!(removed);
        let scheduler_after_down: bool = sqlx::query_scalar(
            "SELECT active FROM public.candidate_scheduler_control
              WHERE control_key = 'scheduler'",
        )
        .fetch_one(&owner)
        .await?;
        assert!(!scheduler_after_down);
        MIGRATOR.run(&owner).await?;
        MIGRATOR.run(&owner).await?;
        let reapplied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.candidate_universe_registry",
        )
        .fetch_one(&owner)
        .await?;
        assert_eq!(reapplied, 2);
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("candidate multi-universe contract FAILED: {error}");
    }
}

/// A correction supersedes the old feed row, but the old succeeded run is
/// still a frozen run-set member and must retain its six exact attributions.
#[tokio::test]
async fn candidate_attributions_survive_same_universe_correction() {
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
        let service_user_id: Uuid = sqlx::query_scalar(
            "SELECT service_user_id FROM public.candidate_scheduler_control
              WHERE control_key = 'scheduler'",
        )
        .fetch_one(&owner)
        .await?;
        let scoring_sha256: String = sqlx::query_scalar(
            "SELECT content_sha256 FROM public.candidate_scoring_configs
              WHERE version = 'candidate-score-v1'",
        )
        .fetch_one(&owner)
        .await?;
        let contract_reference = "candidate-0045-attribution";
        let available_at = "2026-08-14 08:00:00+00";
        let retrieved_at = "2026-08-14 08:01:00+00";
        let cutoff_at = "2026-08-14 10:00:00+00";
        let source_revision = "attribution-test";
        let license_hash = "a".repeat(64);
        let instrument_ids = [
            "910000045.KRX",
            "910000046.KRX",
            "910000047.KRX",
            "910000048.KRX",
            "910000049.KRX",
        ];
        let instrument_id = instrument_ids[0];
        let price_dataset_id = Uuid::new_v4();
        let universe_dataset_id = Uuid::new_v4();
        let status_dataset_id = Uuid::new_v4();
        let flow_dataset_id = Uuid::new_v4();
        let fundamental_dataset_id = Uuid::new_v4();
        let sector_dataset_id = Uuid::new_v4();
        let price_manifest = "1".repeat(64);
        let universe_manifest = "2".repeat(64);
        let status_manifest = "3".repeat(64);
        let flow_manifest = "4".repeat(64);
        let fundamental_manifest = "5".repeat(64);
        let sector_manifest = "6".repeat(64);
        let entitlement_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.data_entitlements
                (contract_document_sha256, contract_reference, status,
                 covered_datasets, covered_uses, effective_from, effective_until, managed_by)
             VALUES ($1, $2, 'ACTIVE',
                     '[\"krx_eod_bars\",\"krx_kospi200_membership\",\
                       \"krx_market_status\",\"krx_investor_flows\",\
                       \"krx_fundamentals\",\"krx_sector_classification\"]'::jsonb,
                     '[\"candidate\"]'::jsonb, DATE '2026-01-01', DATE '2026-12-31', $3)
             RETURNING id",
        )
        .bind(&license_hash)
        .bind(contract_reference)
        .bind(service_user_id)
        .fetch_one(&owner)
        .await?;
        let dataset_versions = [
            (
                price_dataset_id,
                "krx_eod_bars",
                "attribution-price",
                price_manifest.as_str(),
            ),
            (
                universe_dataset_id,
                "krx_kospi200_membership",
                "attribution-universe",
                universe_manifest.as_str(),
            ),
            (
                status_dataset_id,
                "krx_market_status",
                "attribution-status",
                status_manifest.as_str(),
            ),
            (
                flow_dataset_id,
                "krx_investor_flows",
                "attribution-flow",
                flow_manifest.as_str(),
            ),
            (
                fundamental_dataset_id,
                "krx_fundamentals",
                "attribution-fundamental",
                fundamental_manifest.as_str(),
            ),
            (
                sector_dataset_id,
                "krx_sector_classification",
                "attribution-sector",
                sector_manifest.as_str(),
            ),
        ];
        for (id, dataset_id, version, manifest_sha256) in dataset_versions {
            sqlx::query(
                "INSERT INTO public.dataset_versions
                    (id, dataset_id, version, status, manifest_sha256, storage_path)
                 VALUES ($1, $2, $3, 'READY', $4, $5)",
            )
            .bind(id)
            .bind(dataset_id)
            .bind(version)
            .bind(manifest_sha256)
            .bind(format!("db://candidate/{dataset_id}/{version}"))
            .execute(&owner)
            .await?;
        }
        for candidate_instrument_id in instrument_ids {
            sqlx::query(
                "INSERT INTO public.instruments
                    (id, symbol, venue, currency, name, asset_class, status, listed_at)
                 VALUES ($1, $2, 'KRX', 'KRW', '0045 attribution',
                         'EQUITY', 'ACTIVE', DATE '2020-01-01')",
            )
            .bind(candidate_instrument_id)
            .bind(candidate_instrument_id.trim_end_matches(".KRX"))
            .execute(&owner)
            .await?;
        }

        let mut source_tx = owner.begin().await?;
        let universe_snapshot_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.candidate_universe_snapshots
                (index_id, as_of_date, dataset_version_id, manifest_sha256, provider,
                 entitlement_id, entitlement_date, license_ref, source_revision,
                 available_at, retrieved_at, member_count)
             VALUES ('kospi200', DATE '2026-08-14', $1, $2, 'krx', $3, DATE '2026-08-14',
                     $4, $5, $6::timestamptz, $7::timestamptz, 1)
             RETURNING id",
        )
        .bind(universe_dataset_id)
        .bind(&universe_manifest)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(source_revision)
        .bind(available_at)
        .bind(retrieved_at)
        .fetch_one(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_universe_members
                (universe_snapshot_id, instrument_id, announced_at, effective_from,
                 effective_until, available_at, source_revision)
             VALUES ($1, $2, TIMESTAMPTZ '2026-08-01 08:00:00+00', DATE '2026-08-14',
                     NULL, $3::timestamptz, $4)",
        )
        .bind(universe_snapshot_id)
        .bind(instrument_id)
        .bind(available_at)
        .bind(source_revision)
        .execute(&mut *source_tx)
        .await?;
        let flow_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.candidate_investor_flows
                (instrument_id, trade_date, investor_class, net_amount, net_volume,
                 provider, source_revision, available_at)
             VALUES ($1, DATE '2026-08-14', 'FOREIGN', 1, 1, 'krx', $2, $3::timestamptz)
             RETURNING id",
        )
        .bind(instrument_id)
        .bind(source_revision)
        .bind(available_at)
        .fetch_one(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_investor_flow_snapshot_rows
                (dataset_version_id, flow_observation_id, entitlement_id,
                 entitlement_date, license_ref, retrieved_at, manifest_sha256)
             VALUES ($1, $2, $3, DATE '2026-08-14', $4, $5::timestamptz, $6)",
        )
        .bind(flow_dataset_id)
        .bind(flow_id)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(retrieved_at)
        .bind(&flow_manifest)
        .execute(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_market_status_observations
                (instrument_id, trade_date, provider, entitlement_id, entitlement_date,
                 license_ref, source_revision, available_at, retrieved_at,
                 dataset_version_id, manifest_sha256)
             VALUES ($1, DATE '2026-08-14', 'krx', $2, DATE '2026-08-14', $3, $4,
                     $5::timestamptz, $6::timestamptz, $7, $8)",
        )
        .bind(instrument_id)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(source_revision)
        .bind(available_at)
        .bind(retrieved_at)
        .bind(status_dataset_id)
        .bind(&status_manifest)
        .execute(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_fundamental_observations
                (instrument_id, fiscal_period_start, fiscal_period_end, period_kind,
                 statement_scope, metric, value, disclosed_at, available_at, retrieved_at,
                 provider, entitlement_id, entitlement_date, license_ref, source_revision,
                 dataset_version_id, manifest_sha256)
             VALUES ($1, DATE '2024-01-01', DATE '2024-06-30', 'HALF', 'CONSOLIDATED',
                     'revenue', 1, TIMESTAMPTZ '2024-08-01 00:00:00+00',
                     $2::timestamptz, $3::timestamptz, 'krx', $4, DATE '2026-08-14',
                     $5, $6, $7, $8)",
        )
        .bind(instrument_id)
        .bind(available_at)
        .bind(retrieved_at)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(source_revision)
        .bind(fundamental_dataset_id)
        .bind(&fundamental_manifest)
        .execute(&mut *source_tx)
        .await?;
        let sector_version_id: Uuid = sqlx::query_scalar(
            "INSERT INTO public.candidate_sector_versions
                (taxonomy_id, taxonomy_version, effective_from, available_at, retrieved_at,
                 provider, entitlement_id, entitlement_date, license_ref, source_revision,
                 dataset_version_id, manifest_sha256)
             VALUES ('krx-sector', 'attribution-v1', DATE '2026-01-01', $1::timestamptz,
                     $2::timestamptz, 'krx', $3, DATE '2026-08-14', $4, $5, $6, $7)
             RETURNING id",
        )
        .bind(available_at)
        .bind(retrieved_at)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(source_revision)
        .bind(sector_dataset_id)
        .bind(&sector_manifest)
        .fetch_one(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_price_publications
                (dataset_version_id, dataset_version, manifest_sha256, market,
                 curated_generation, first_session, last_session, provider,
                 entitlement_id, license_ref, source_revision, available_at, retrieved_at)
             VALUES ($1, 'attribution-price', $2, 'kr', 1, DATE '2026-08-14',
                     DATE '2026-08-14', 'krx', $3, $4, $5, $6::timestamptz, $7::timestamptz)",
        )
        .bind(price_dataset_id)
        .bind(&price_manifest)
        .bind(entitlement_id)
        .bind(contract_reference)
        .bind(source_revision)
        .bind(available_at)
        .bind(retrieved_at)
        .execute(&mut *source_tx)
        .await?;

        let old_job_id = Uuid::new_v4();
        let old_run_id = Uuid::new_v4();
        let old_feed_id = Uuid::new_v4();
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(service_user_id.to_string())
            .execute(&mut *source_tx)
            .await?;
        sqlx::query(
            "INSERT INTO public.jobs
                (id, owner_user_id, job_type, status, idempotency_key, payload_json,
                 max_attempts, attempt_count, finished_at)
             VALUES ($1, $2, 'candidate_compute', 'SUCCEEDED',
                     'candidate:scheduled:attribution-old', '{}'::jsonb, 3, 1, $3::timestamptz)",
        )
        .bind(old_job_id)
        .bind(service_user_id)
        .bind(cutoff_at)
        .execute(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.job_attempts
                (job_id, attempt_no, outcome, claimed_by, started_at, finished_at)
             VALUES ($1, 1, 'SUCCEEDED', 'attribution-worker', $2::timestamptz, $3::timestamptz)",
        )
        .bind(old_job_id)
        .bind(available_at)
        .bind(cutoff_at)
        .execute(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.stock_analysis_runs
                (id, as_of_date, cutoff_at, computation_seq, status, job_id,
                 scoring_config_version, scoring_config_sha256, universe_snapshot_id,
                 universe_key, universe_entitlement_id, price_dataset_version_id,
                 price_entitlement_id, price_curated_version, price_manifest_sha256,
                 status_dataset_version_id, status_entitlement_id, status_manifest_sha256,
                 flow_dataset_version_id, flow_entitlement_id, flow_manifest_sha256,
                 fundamental_dataset_version_id, fundamental_entitlement_id,
                 fundamental_manifest_sha256, sector_version_id, sector_entitlement_id,
                 input_identity_sha256, summary_json, published_at)
             VALUES ($1, DATE '2026-08-14', $2::timestamptz, 1, 'SUCCEEDED', $3,
                     'candidate-score-v1', $4, $5, 'kospi200', $6, $7, $6, 1, $8,
                     $9, $6, $10, $11, $6, $12, $13, $6, $14, $15, $6, $16,
                     '{}'::jsonb, $2::timestamptz)",
        )
        .bind(old_run_id)
        .bind(cutoff_at)
        .bind(old_job_id)
        .bind(&scoring_sha256)
        .bind(universe_snapshot_id)
        .bind(entitlement_id)
        .bind(price_dataset_id)
        .bind(&price_manifest)
        .bind(status_dataset_id)
        .bind(&status_manifest)
        .bind(flow_dataset_id)
        .bind(&flow_manifest)
        .bind(fundamental_dataset_id)
        .bind(&fundamental_manifest)
        .bind(sector_version_id)
        .bind("a".repeat(64))
        .execute(&mut *source_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_feed_snapshots
                (id, run_id, universe_key, as_of_date, computation_seq, status, published_at)
             VALUES ($1, $2, 'kospi200', DATE '2026-08-14', 1, 'PUBLISHED', $3::timestamptz)",
        )
        .bind(old_feed_id)
        .bind(old_run_id)
        .bind(cutoff_at)
        .execute(&mut *source_tx)
        .await?;
        for (rank, candidate_instrument_id) in instrument_ids.iter().enumerate() {
            let snapshot_id = Uuid::new_v4();
            let rank = i32::try_from(rank + 1).expect("small attribution rank");
            sqlx::query(
                "INSERT INTO public.stock_analysis_snapshots
                    (id, run_id, instrument_id, sector_code, fundamental_profile,
                     eligible, exclusion_codes, flow_score, fundamental_score,
                     technical_score, total_score, flow_coverage, fundamental_coverage,
                     technical_coverage, evidence_strength, rank, normalization_scope,
                     factors_json, scenarios_json, provenance_json, content_sha256)
                 VALUES ($1, $2, $3, 'TECH', 'non_financial', true, '[]'::jsonb,
                         1, 1, 1, $4, 1, 1, 1, 'STRONG', $5, 'SECTOR',
                         '{}'::jsonb,
                         '{\"bullish\":{},\"neutral\":{},\"bearish\":{}}'::jsonb,
                         '{\"input_identity_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"as_of_date\":\"2026-08-14\"}'::jsonb,
                         $6)",
            )
            .bind(snapshot_id)
            .bind(old_run_id)
            .bind(*candidate_instrument_id)
            .bind(rank)
            .bind(rank)
            .bind("a".repeat(64))
            .execute(&mut *source_tx)
            .await?;
            sqlx::query(
                "INSERT INTO public.candidate_feed_items
                    (feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(old_feed_id)
            .bind(old_run_id)
            .bind(snapshot_id)
            .bind(*candidate_instrument_id)
            .bind(rank)
            .execute(&mut *source_tx)
            .await?;
        }
        source_tx.commit().await?;

        let app = role_pool(&super_url, &db, "app").await?;
        let old_before_correction: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.candidate_published_source_attributions($1)",
        )
        .bind(old_run_id)
        .fetch_one(&app)
        .await?;
        assert_eq!(old_before_correction, 6);
        drop(app);

        let mut correction_tx = owner.begin().await?;
        let correction_job_id = Uuid::new_v4();
        let correction_run_id = Uuid::new_v4();
        let correction_feed_id = Uuid::new_v4();
        sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
            .bind(service_user_id.to_string())
            .execute(&mut *correction_tx)
            .await?;
        sqlx::query(
            "INSERT INTO public.jobs
                (id, owner_user_id, job_type, status, idempotency_key, payload_json,
                 max_attempts, attempt_count, finished_at)
             VALUES ($1, $2, 'candidate_compute', 'SUCCEEDED',
                     'candidate:scheduled:attribution-correction', '{}'::jsonb,
                     3, 1, $3::timestamptz)",
        )
        .bind(correction_job_id)
        .bind(service_user_id)
        .bind(cutoff_at)
        .execute(&mut *correction_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.job_attempts
                (job_id, attempt_no, outcome, claimed_by, started_at, finished_at)
             VALUES ($1, 1, 'SUCCEEDED', 'attribution-worker', $2::timestamptz, $3::timestamptz)",
        )
        .bind(correction_job_id)
        .bind(available_at)
        .bind(cutoff_at)
        .execute(&mut *correction_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.stock_analysis_runs
                (id, as_of_date, cutoff_at, computation_seq, status, job_id,
                 scoring_config_version, scoring_config_sha256, universe_snapshot_id,
                 universe_key, universe_entitlement_id, price_dataset_version_id,
                 price_entitlement_id, price_curated_version, price_manifest_sha256,
                 status_dataset_version_id, status_entitlement_id, status_manifest_sha256,
                 flow_dataset_version_id, flow_entitlement_id, flow_manifest_sha256,
                 fundamental_dataset_version_id, fundamental_entitlement_id,
                 fundamental_manifest_sha256, sector_version_id, sector_entitlement_id,
                 input_identity_sha256, summary_json, published_at)
             VALUES ($1, DATE '2026-08-14', $2::timestamptz, 2, 'SUCCEEDED', $3,
                     'candidate-score-v1', $4, $5, 'kospi200', $6, $7, $6, 1, $8,
                     $9, $6, $10, $11, $6, $12, $13, $6, $14, $15, $6, $16,
                     '{}'::jsonb, $2::timestamptz)",
        )
        .bind(correction_run_id)
        .bind(cutoff_at)
        .bind(correction_job_id)
        .bind(&scoring_sha256)
        .bind(universe_snapshot_id)
        .bind(entitlement_id)
        .bind(price_dataset_id)
        .bind(&price_manifest)
        .bind(status_dataset_id)
        .bind(&status_manifest)
        .bind(flow_dataset_id)
        .bind(&flow_manifest)
        .bind(fundamental_dataset_id)
        .bind(&fundamental_manifest)
        .bind(sector_version_id)
        .bind("b".repeat(64))
        .execute(&mut *correction_tx)
        .await?;
        sqlx::query(
            "UPDATE public.candidate_feed_snapshots
                SET status = 'SUPERSEDED', superseded_by = $1
              WHERE id = $2",
        )
        .bind(correction_feed_id)
        .bind(old_feed_id)
        .execute(&mut *correction_tx)
        .await?;
        sqlx::query(
            "INSERT INTO public.candidate_feed_snapshots
                (id, run_id, universe_key, as_of_date, computation_seq, status, published_at)
             VALUES ($1, $2, 'kospi200', DATE '2026-08-14', 2, 'PUBLISHED', $3::timestamptz)",
        )
        .bind(correction_feed_id)
        .bind(correction_run_id)
        .bind(cutoff_at)
        .execute(&mut *correction_tx)
        .await?;
        for (rank, candidate_instrument_id) in instrument_ids.iter().enumerate() {
            let snapshot_id = Uuid::new_v4();
            let rank = i32::try_from(rank + 1).expect("small attribution rank");
            sqlx::query(
                "INSERT INTO public.stock_analysis_snapshots
                    (id, run_id, instrument_id, sector_code, fundamental_profile,
                     eligible, exclusion_codes, flow_score, fundamental_score,
                     technical_score, total_score, flow_coverage, fundamental_coverage,
                     technical_coverage, evidence_strength, rank, normalization_scope,
                     factors_json, scenarios_json, provenance_json, content_sha256)
                 VALUES ($1, $2, $3, 'TECH', 'non_financial', true, '[]'::jsonb,
                         1, 1, 1, $4, 1, 1, 1, 'STRONG', $5, 'SECTOR',
                         '{}'::jsonb,
                         '{\"bullish\":{},\"neutral\":{},\"bearish\":{}}'::jsonb,
                         '{\"input_identity_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"as_of_date\":\"2026-08-14\"}'::jsonb,
                         $6)",
            )
            .bind(snapshot_id)
            .bind(correction_run_id)
            .bind(*candidate_instrument_id)
            .bind(rank)
            .bind(rank)
            .bind("b".repeat(64))
            .execute(&mut *correction_tx)
            .await?;
            sqlx::query(
                "INSERT INTO public.candidate_feed_items
                    (feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(correction_feed_id)
            .bind(correction_run_id)
            .bind(snapshot_id)
            .bind(*candidate_instrument_id)
            .bind(rank)
            .execute(&mut *correction_tx)
            .await?;
        }
        let pending_run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO public.stock_analysis_runs
                (id, as_of_date, cutoff_at, computation_seq, status,
                 scoring_config_version, scoring_config_sha256, universe_snapshot_id,
                 universe_key, universe_entitlement_id, price_dataset_version_id,
                 price_entitlement_id, price_curated_version, price_manifest_sha256,
                 status_dataset_version_id, status_entitlement_id, status_manifest_sha256,
                 flow_dataset_version_id, flow_entitlement_id, flow_manifest_sha256,
                 fundamental_dataset_version_id, fundamental_entitlement_id,
                 fundamental_manifest_sha256, sector_version_id, sector_entitlement_id,
                 input_identity_sha256)
             VALUES ($1, DATE '2026-08-14', $2::timestamptz, 3, 'PENDING',
                     'candidate-score-v1', $3, $4, 'kospi200', $5, $6, $5, 1, $7,
                     $8, $5, $9, $10, $5, $11, $12, $5, $13, $14, $5, $15)",
        )
        .bind(pending_run_id)
        .bind(cutoff_at)
        .bind(&scoring_sha256)
        .bind(universe_snapshot_id)
        .bind(entitlement_id)
        .bind(price_dataset_id)
        .bind(&price_manifest)
        .bind(status_dataset_id)
        .bind(&status_manifest)
        .bind(flow_dataset_id)
        .bind(&flow_manifest)
        .bind(fundamental_dataset_id)
        .bind(&fundamental_manifest)
        .bind(sector_version_id)
        .bind("c".repeat(64))
        .execute(&mut *correction_tx)
        .await?;
        correction_tx.commit().await?;

        let app = role_pool(&super_url, &db, "app").await?;
        let attribution_counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                (SELECT count(*) FROM public.candidate_published_source_attributions($1)),
                (SELECT count(*) FROM public.candidate_published_source_attributions($2)),
                (SELECT count(*) FROM public.candidate_published_source_attributions($3)),
                (SELECT count(*) FROM public.candidate_published_source_attributions($4))",
        )
        .bind(old_run_id)
        .bind(correction_run_id)
        .bind(pending_run_id)
        .bind(Uuid::new_v4())
        .fetch_one(&app)
        .await?;
        assert_eq!(attribution_counts, (6, 6, 0, 0));
        let feed_status: String =
            sqlx::query_scalar("SELECT status FROM public.candidate_feed_snapshots WHERE id = $1")
                .bind(old_feed_id)
                .fetch_one(&owner)
                .await?;
        assert_eq!(feed_status, "SUPERSEDED");
        drop(app);
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("candidate attribution correction contract FAILED: {error}");
    }
}

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
        "paper_settlement_outbox_claim_state_check",
        "claim_expires_at",
        "no increments beyond max_attempts",
        "at-least-once",
        "notification_deliveries_delivery_lease_check",
        "tenant_all_app_pending_targets",
        "tenant_all_owner_pending_targets",
        "tenant_all_app_recommendation_runs",
        "tenant_all_owner_recommendation_runs",
        "tenant_all_app_notification_deliveries",
        "tenant_all_owner_notification_deliveries",
        "NULLIF(",
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
    for sql in [
        PAPER_SETTLEMENT_OUTBOX_UP_SQL,
        PAPER_SETTLEMENT_OUTBOX_DOWN_SQL,
    ] {
        assert!(
            !sql.contains("current_setting('app.actor_user_id', true)::uuid"),
            "0041 must not cast an empty actor GUC directly to uuid"
        );
    }
    for token in [
        "Paper settlement rollback blocked while pending outbox obligations exist",
        "terminal target without durable obligation",
        "DROP TABLE public.paper_settlement_outbox_archive",
        "DROP FUNCTION IF EXISTS public.enqueue_paper_settlement_outbox",
        "DROP CONSTRAINT IF EXISTS notification_deliveries_notification_channel_uq",
        "DROP CONSTRAINT IF EXISTS notification_deliveries_notification_owner_fk",
        "DROP POLICY IF EXISTS tenant_all_app_recommendation_runs",
        "DROP POLICY IF EXISTS tenant_all_owner_recommendation_runs",
        "DROP POLICY IF EXISTS tenant_all_app_notification_deliveries",
        "DROP POLICY IF EXISTS tenant_all_owner_notification_deliveries",
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
        "dataset.version",
        "FOR SHARE OF run, config",
        "recommendation_items",
        "owner_user_id = $2",
    ] {
        assert!(
            settlement.contains(token),
            "exact settlement lineage lost {token}"
        );
    }
    assert!(
        !settlement.contains("FOR SHARE OF run, config, dataset"),
        "app parity must not row-lock the read-only dataset_versions table"
    );
    assert!(
        !settlement.contains("dataset.dataset_id || '@' || dataset.version"),
        "parity must retain production's plain dataset.version representation"
    );
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
        "claim_paper_settlement_outbox",
        "claim_token",
    ] {
        assert!(
            PAPER_PENDING_TARGET_REPO_RS.contains(token),
            "repository recovery seam is missing {token}"
        );
    }
}

#[test]
fn paper_announcement_claim_binds_postgres_integer_limit() {
    for token in [
        "pub async fn due_announcements_worker(",
        "limit: i32",
        ".bind(limit.clamp(1, 1000))",
    ] {
        assert!(
            PAPER_PENDING_TARGET_REPO_RS.contains(token),
            "announcement claim boundary is missing {token}"
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
    // The migration backfill creates another due row for the terminal target
    // above.  Move every unrelated pending obligation out of this bounded
    // claim window so the race below proves single-lease ownership of this
    // exact recovery row rather than depending on global queue cardinality.
    sqlx::query(
        "UPDATE paper_settlement_outbox \
            SET available_at = CASE WHEN id = $1 THEN now() \
                                    ELSE now() + interval '1 hour' END, \
                claim_token = NULL, claim_expires_at = NULL \
          WHERE delivered_at IS NULL AND exhausted_at IS NULL",
    )
    .bind(recovery_outbox_id)
    .execute(&owner_actor)
    .await?;
    let claim_worker_a = effective_role_pool(super_url, db, "worker", None, 2).await?;
    let claim_worker_b = effective_role_pool(super_url, db, "worker", None, 2).await?;
    let claim_a = async {
        sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT id, claim_token FROM claim_paper_settlement_outbox(1, 60)",
        )
        .fetch_optional(&claim_worker_a)
        .await
    };
    let claim_b = async {
        sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT id, claim_token FROM claim_paper_settlement_outbox(1, 60)",
        )
        .fetch_optional(&claim_worker_b)
        .await
    };
    let (claim_a, claim_b) = tokio::join!(claim_a, claim_b);
    let claims = [claim_a?, claim_b?];
    assert_eq!(
        claims
            .iter()
            .flatten()
            .filter(|(id, _)| *id == recovery_outbox_id)
            .count(),
        1,
        "concurrent workers must lease this due outbox row only once"
    );
    // Clear every lease returned by this bounded probe before exercising the
    // failure transition; unrelated due rows are valid queue work and must
    // not affect the assertion for the recovery row.
    for (claimed_id, claim_token) in claims.iter().flatten() {
        sqlx::query(
            "UPDATE paper_settlement_outbox \
             SET claim_token = NULL, claim_expires_at = NULL \
             WHERE id = $1 AND claim_token = $2",
        )
        .bind(claimed_id)
        .bind(claim_token)
        .execute(&owner_actor)
        .await?;
    }
    let app_failure_a = app.clone();
    let app_failure_b = app.clone();
    let failure_a = async move {
        sqlx::query_as::<_, (i32, bool)>(
            "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'timeout')",
        )
        .bind(recovery_outbox_id)
        .bind(user_id)
        .fetch_optional(&app_failure_a)
        .await
    };
    let failure_b = async move {
        sqlx::query_as::<_, (i32, bool)>(
            "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'timeout')",
        )
        .bind(recovery_outbox_id)
        .bind(user_id)
        .fetch_optional(&app_failure_b)
        .await
    };
    let (first_failure, second_failure) = tokio::join!(failure_a, failure_b);
    let concurrent_failures = [first_failure?, second_failure?];
    assert_eq!(
        concurrent_failures
            .iter()
            .filter(|failure| failure.is_some())
            .count(),
        1,
        "a locked due check must let only one concurrent runner count a failure"
    );
    assert_eq!(
        concurrent_failures.iter().flatten().next().copied(),
        Some((1, false))
    );
    sqlx::query("UPDATE paper_settlement_outbox SET available_at = now() WHERE id = $1")
        .bind(recovery_outbox_id)
        .execute(&owner_actor)
        .await?;
    let second_failure: Option<(i32, bool)> = sqlx::query_as(
        "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'timeout')",
    )
    .bind(recovery_outbox_id)
    .bind(user_id)
    .fetch_optional(&app)
    .await?;
    assert_eq!(second_failure, Some((2, true)));
    sqlx::query("UPDATE paper_settlement_outbox SET available_at = now() WHERE id = $1")
        .bind(recovery_outbox_id)
        .execute(&owner_actor)
        .await?;
    let post_exhaustion: Option<(i32, bool)> = sqlx::query_as(
        "SELECT attempts, exhausted FROM fail_paper_settlement_outbox($1, $2, 'late-timeout')",
    )
    .bind(recovery_outbox_id)
    .bind(user_id)
    .fetch_optional(&app)
    .await?;
    assert_eq!(
        post_exhaustion,
        Some((2, true)),
        "terminal failure reports must not increment beyond max_attempts"
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
    let down_error = MIGRATOR.undo(&mut *guarded, 40).await.unwrap_err();
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
    MIGRATOR.undo(owner, 40).await?;
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
        "auth_audit_log_select_migration_owner",
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
        "DROP POLICY IF EXISTS auth_audit_log_select_migration_owner",
        "undelivered outbox obligations",
        "auth audit rollback blocked while undelivered outbox obligations exist",
        "USING ERRCODE = '55000'",
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
    for (name, sql) in [
        ("0039", AUTH_AUDIT_OUTBOX_UP_SQL),
        ("0040", IDENTITY_PROVISIONING_UP_SQL),
        ("0041", PAPER_SETTLEMENT_OUTBOX_UP_SQL),
    ] {
        for forbidden in [
            "pg_catalog.nullif",
            "pg_catalog.coalesce",
            "pg_catalog.extract",
        ] {
            assert!(
                !sql.contains(forbidden),
                "{name} must use PostgreSQL special form {forbidden}, not a nonexistent catalog function"
            );
        }
    }
    let guard = AUTH_AUDIT_OUTBOX_DOWN_SQL
        .find("auth audit rollback blocked while undelivered outbox obligations exist")
        .expect("0039 rollback guard");
    let drop_table = AUTH_AUDIT_OUTBOX_DOWN_SQL
        .find("DROP TABLE public.auth_audit_outbox")
        .expect("0039 rollback drop");
    assert!(
        guard < drop_table,
        "0039 must guard before dropping the outbox"
    );
}

/// DB-gated rollback probe for 0039.  A pending event blocks the destructive
/// down migration; once the durable copy is marked delivered, the same down
/// migration is allowed to remove the bounded outbox table.
#[tokio::test]
async fn auth_audit_rollback_guard_is_fail_closed_and_allows_delivered_rows() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = async {
        MIGRATOR.run_to(39, &owner).await?;
        let event_id: Uuid = sqlx::query_scalar(
            "INSERT INTO auth_audit_outbox \
             (event_key, action, created_at) \
             VALUES ('rollback-guard-probe', 'auth.test', now()) \
             RETURNING id",
        )
        .fetch_one(&owner)
        .await?;
        let blocked = MIGRATOR.undo(&owner, 38).await.unwrap_err();
        assert_eq!(migrate_pg_code(&blocked).as_deref(), Some("55000"));
        sqlx::query("UPDATE auth_audit_outbox SET delivered_at = now() WHERE id = $1")
            .bind(event_id)
            .execute(&owner)
            .await?;
        MIGRATOR.undo(&owner, 38).await?;
        let gone: bool =
            sqlx::query_scalar("SELECT to_regclass('public.auth_audit_outbox') IS NULL")
                .fetch_one(&owner)
                .await?;
        assert!(gone, "delivered outbox rows permit a safe rollback");
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("auth audit rollback guard FAILED: {error}");
    }
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
        "identity_actor_capabilities",
        "authenticate_identity_actor",
        "consume_identity_actor_capability",
        "transaction_id",
        "backend_pid",
        "p_actor_capability",
        "p_invite_hash",
        "NO FORCE ROW LEVEL SECURITY",
        "FORCE ROW LEVEL SECURITY",
        "EXTRACT(EPOCH FROM pg_catalog.clock_timestamp())",
        "REVOKE INSERT, UPDATE, DELETE ON TABLE public.invitations FROM app",
        "GRANT SELECT ON TABLE public.invitations TO app",
    ] {
        assert!(
            IDENTITY_PROVISIONING_UP_SQL.contains(token),
            "0040 identity up is missing {token}"
        );
    }
    assert!(
        !IDENTITY_PROVISIONING_UP_SQL.contains("pg_catalog.extract"),
        "0040 must use PostgreSQL's EXTRACT special form, not a nonexistent catalog function"
    );
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
            && !IDENTITY_PROVISIONING_UP_SQL.contains("GRANT INSERT ON TABLE public.user_roles")
            && !IDENTITY_PROVISIONING_UP_SQL
                .contains("GRANT INSERT, UPDATE, DELETE ON TABLE public.invitations TO app"),
        "identity provisioning must not add table write grants"
    );
    assert!(
        IDENTITY_PROVISIONING_DOWN_SQL
            .contains("GRANT INSERT, UPDATE, DELETE ON TABLE public.invitations TO app"),
        "0040 down must restore the 0009 invitation DML grants"
    );
    for token in [
        "DROP FUNCTION public.bind_redeemed_identity",
        "DROP FUNCTION public.claim_invitation(uuid, uuid, text, text, text)",
        "DROP FUNCTION public.expire_pending_invitations",
        "DROP FUNCTION public.create_invitation(uuid, text, text, text, bigint, uuid)",
        "authenticate_identity_actor",
        "identity_actor_capabilities",
        "DROP COLUMN role_id",
        "DROP COLUMN provisioned_by_user_id",
        "DROP INDEX public.invitations_pending_email_uq",
        "cannot roll back identity provisioning while provisional identities exist",
        "cannot roll back identity provisioning while Owner invitations remain",
        "WHERE role_id <> 'member'",
    ] {
        assert!(
            IDENTITY_PROVISIONING_DOWN_SQL.contains(token),
            "0040 down is missing {token}"
        );
    }
    for signature in [
        "public.create_invitation(uuid, text, text, text, bigint, uuid)",
        "public.claim_invitation(uuid, uuid, text, text, text)",
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

async fn create_invitation_with_capability(
    app: &PgPool,
    owner_id: Uuid,
    session_hash: &str,
    email: &str,
    role: &str,
    invite_hash: &str,
    expires_at: i64,
) -> Result<Uuid, sqlx::Error> {
    let mut tx = app.begin().await?;
    let capability: Uuid = sqlx::query_scalar("SELECT public.authenticate_identity_actor($1, $2)")
        .bind(owner_id)
        .bind(session_hash)
        .fetch_one(&mut *tx)
        .await?;
    let invitation_id: Uuid =
        sqlx::query_scalar("SELECT public.create_invitation($1, $2, $3, $4, $5, $6)")
            .bind(owner_id)
            .bind(email)
            .bind(role)
            .bind(invite_hash)
            .bind(expires_at)
            .bind(capability)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(invitation_id)
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

    let owner_session_hash = "11".repeat(32);
    let other_owner_session_hash = "22".repeat(32);
    let owner_actor = actor_pool(super_url, db, "migration_owner", &owner_id.to_string()).await?;
    let other_owner_actor = actor_pool(
        super_url,
        db,
        "migration_owner",
        &other_owner_id.to_string(),
    )
    .await?;
    for (actor_pool, user_id, session_hash, csrf_hash) in [
        (
            &owner_actor,
            owner_id,
            owner_session_hash.as_str(),
            "33".repeat(32),
        ),
        (
            &other_owner_actor,
            other_owner_id,
            other_owner_session_hash.as_str(),
            "44".repeat(32),
        ),
    ] {
        sqlx::query(
            "INSERT INTO web_sessions \
             (user_id, session_hash, csrf_hash, expires_at) \
             VALUES ($1, $2, $3, now() + interval '1 hour')",
        )
        .bind(user_id)
        .bind(session_hash)
        .bind(csrf_hash)
        .execute(actor_pool)
        .await?;
    }

    let app = role_pool(super_url, db, "app").await?;
    let admin = role_pool(super_url, db, "admin").await?;
    let audit = role_pool(super_url, db, "audit_writer").await?;
    let app_identity: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&app)
        .await?;
    assert_eq!(app_identity.0, "app");
    assert_eq!(
        app_identity.0, app_identity.1,
        "production identity probes must use a direct app login, not an elevated role"
    );
    let invitation_privileges: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege('app', 'public.invitations', 'SELECT'), \
                has_table_privilege('app', 'public.invitations', 'INSERT'), \
                has_table_privilege('app', 'public.invitations', 'UPDATE'), \
                has_table_privilege('app', 'public.invitations', 'DELETE')",
    )
    .fetch_one(owner)
    .await?;
    assert_eq!(
        invitation_privileges,
        (true, false, false, false),
        "app may inspect invitations but direct DML requires the capability seam"
    );
    let expires_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64 + 3_600;
    let owner_invitation_id = create_invitation_with_capability(
        &app,
        owner_id,
        &owner_session_hash,
        "owner-rollback@example.test",
        "owner",
        &"0f".repeat(32),
        expires_at,
    )
    .await?;
    let invite_hash = "a1".repeat(32);
    let invitation_id = create_invitation_with_capability(
        &app,
        owner_id,
        &owner_session_hash,
        "new-member@example.test",
        "member",
        &invite_hash,
        expires_at,
    )
    .await?;
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM auth_audit_outbox \
         WHERE event_key = $1 AND action = 'auth.invite_created'",
    )
    .bind(format!("invite:{invitation_id}:created"))
    .fetch_one(owner)
    .await?;
    assert_eq!(outbox_count, 1, "invite mutation must enqueue atomically");

    // The actor GUC is only a tenant selector, never a write authorization
    // boundary.  Exercise every invitation DML verb through direct app
    // logins, explicitly setting the actor in the transaction.  Each failed
    // statement is rolled back so the next probe uses a clean transaction.
    let app_owner_actor = actor_pool(super_url, db, "app", &owner_id.to_string()).await?;
    let app_other_actor = actor_pool(super_url, db, "app", &other_owner_id.to_string()).await?;
    let mut acl_tx = app_owner_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_insert_own = sqlx::query(
        "INSERT INTO invitations (user_id, email, invite_hash, status, expires_at, role_id) \
         VALUES ($1, $2, $3, 'PENDING', now() + interval '1 hour', 'member')",
    )
    .bind(owner_id)
    .bind("acl-own@example.test")
    .bind("ab".repeat(32))
    .execute(&mut *acl_tx)
    .await
    .expect_err("direct app INSERT must be denied for its own tenant");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_insert_own).as_deref(), Some("42501"));

    let mut acl_tx = app_owner_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_insert_cross = sqlx::query(
        "INSERT INTO invitations (user_id, email, invite_hash, status, expires_at, role_id) \
         VALUES ($1, $2, $3, 'PENDING', now() + interval '1 hour', 'member')",
    )
    .bind(other_owner_id)
    .bind("acl-cross@example.test")
    .bind("ac".repeat(32))
    .execute(&mut *acl_tx)
    .await
    .expect_err("direct app INSERT must be denied across tenants");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_insert_cross).as_deref(), Some("42501"));

    let mut acl_tx = app_owner_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_update_own = sqlx::query("UPDATE invitations SET email = $1 WHERE id = $2")
        .bind("acl-update-own@example.test")
        .bind(invitation_id)
        .execute(&mut *acl_tx)
        .await
        .expect_err("direct app UPDATE must be denied for its own tenant");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_update_own).as_deref(), Some("42501"));

    let mut acl_tx = app_other_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(other_owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_update_cross = sqlx::query("UPDATE invitations SET email = $1 WHERE id = $2")
        .bind("acl-update-cross@example.test")
        .bind(invitation_id)
        .execute(&mut *acl_tx)
        .await
        .expect_err("direct app UPDATE must be denied across tenants");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_update_cross).as_deref(), Some("42501"));

    let mut acl_tx = app_owner_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_delete_own = sqlx::query("DELETE FROM invitations WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *acl_tx)
        .await
        .expect_err("direct app DELETE must be denied for its own tenant");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_delete_own).as_deref(), Some("42501"));

    let mut acl_tx = app_other_actor.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id', $1, true)")
        .bind(other_owner_id.to_string())
        .execute(&mut *acl_tx)
        .await?;
    let acl_delete_cross = sqlx::query("DELETE FROM invitations WHERE id = $1")
        .bind(owner_invitation_id)
        .execute(&mut *acl_tx)
        .await
        .expect_err("direct app DELETE must be denied across tenants");
    acl_tx.rollback().await?;
    assert_eq!(pg_code(&acl_delete_cross).as_deref(), Some("42501"));

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
    let delivered: (i32, i32) = sqlx::query_as(
        "SELECT delivered_count, failed_count FROM public.deliver_auth_audit_batch(64)",
    )
    .fetch_one(&audit)
    .await?;
    assert!(delivered.0 >= 2);
    assert_eq!(delivered.1, 0, "contract audit delivery must not fail");
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

    let duplicate = create_invitation_with_capability(
        &app,
        owner_id,
        &owner_session_hash,
        "new-member@example.test",
        "member",
        &"b2".repeat(32),
        expires_at,
    )
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
    .fetch_one(&owner_actor)
    .await?;
    let replacement_id = create_invitation_with_capability(
        &app,
        other_owner_id,
        &other_owner_session_hash,
        "stale@example.test",
        "member",
        &"e5".repeat(32),
        expires_at,
    )
    .await?;
    assert_ne!(replacement_id, stale_id);
    let stale_status: String = sqlx::query_scalar("SELECT status FROM invitations WHERE id = $1")
        .bind(stale_id)
        .fetch_one(&owner_actor)
        .await?;
    assert_eq!(stale_status, "EXPIRED");

    let forged_owner = create_invitation_with_capability(
        &app,
        member_id,
        &owner_session_hash,
        "forged@example.test",
        "member",
        &"c3".repeat(32),
        expires_at,
    )
    .await
    .unwrap_err();
    assert_eq!(pg_code(&forged_owner).as_deref(), Some("42501"));

    // Direct app callers cannot mint Owner-B's capability with Owner-A's live
    // session, even when they forge the supplied owner UUID and actor GUC.
    let app_a = actor_pool(super_url, db, "app", &owner_id.to_string()).await?;
    let mut forged_tx = app_a.begin().await?;
    let owner_a_capability: Uuid =
        sqlx::query_scalar("SELECT public.authenticate_identity_actor($1, $2)")
            .bind(owner_id)
            .bind(&owner_session_hash)
            .fetch_one(&mut *forged_tx)
            .await?;
    let forged_cross_owner = sqlx::query(
        "SELECT public.create_invitation($1, 'cross-owner@example.test', 'member', $2, $3, $4)",
    )
    .bind(other_owner_id)
    .bind("c4".repeat(32))
    .bind(expires_at)
    .bind(owner_a_capability)
    .fetch_one(&mut *forged_tx)
    .await
    .unwrap_err();
    forged_tx.rollback().await?;
    assert_eq!(pg_code(&forged_cross_owner).as_deref(), Some("42501"));

    let owner_b_invite_hash = "c5".repeat(32);
    let owner_b_invitation = create_invitation_with_capability(
        &app,
        other_owner_id,
        &other_owner_session_hash,
        "owner-b-claim@example.test",
        "member",
        &owner_b_invite_hash,
        expires_at,
    )
    .await?;
    let forged_claim =
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4, $5)")
            .bind(other_owner_id)
            .bind(owner_b_invitation)
            .bind(&owner_b_invite_hash)
            .bind("https://issuer.test")
            .bind("auth0|forged-owner-b")
            .fetch_one(&app_a)
            .await?;
    assert!(
        !forged_claim,
        "Owner-A's invitation capability cannot claim Owner-B"
    );

    // Existing identities are globally ineligible for invitations, including
    // a provisional identity redeemed under a different Owner tenant.
    let existing_user = create_invitation_with_capability(
        &app,
        owner_id,
        &owner_session_hash,
        "provision-member@example.test",
        "member",
        &"f1".repeat(32),
        expires_at,
    )
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
    let provisional_invite = create_invitation_with_capability(
        &app,
        other_owner_id,
        &other_owner_session_hash,
        "provisional-existing@example.test",
        "member",
        &"f2".repeat(32),
        expires_at,
    )
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
    let race_invite_hash = "ab".repeat(32);
    let race_invitation_id = create_invitation_with_capability(
        &app,
        owner_id,
        &owner_session_hash,
        &race_email,
        "member",
        &race_invite_hash,
        expires_at,
    )
    .await?;
    let race_create_pool = role_pool(super_url, db, "app").await?;
    let race_claim_pool = role_pool(super_url, db, "app").await?;
    let race_email_for_create = race_email.clone();
    let race_session_hash = owner_session_hash.clone();
    let create_race = async move {
        // The race loser must still fail at the global unique invitation
        // boundary, but it needs its own authenticated Owner capability.
        let mut tx = race_create_pool.begin().await?;
        let capability: Uuid =
            sqlx::query_scalar("SELECT public.authenticate_identity_actor($1, $2)")
                .bind(owner_id)
                .bind(&race_session_hash)
                .fetch_one(&mut *tx)
                .await?;
        let result = sqlx::query("SELECT public.create_invitation($1, $2, 'member', $3, $4, $5)")
            .bind(owner_id)
            .bind(race_email_for_create)
            .bind("cd".repeat(32))
            .bind(expires_at)
            .bind(capability)
            .fetch_one(&mut *tx)
            .await;
        tx.rollback().await?;
        result
    };
    let claim_race = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4, $5)")
            .bind(owner_id)
            .bind(race_invitation_id)
            .bind(&race_invite_hash)
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
        id: format!("inv-{}", "f3".repeat(32)),
        email: "adapter-member@example.test".to_string(),
        role: Role::Member,
        created_at_secs: expires_at - 3_600,
        expires_at_secs: expires_at,
        redeemed_by: None,
        redeemed_at_secs: None,
    };
    with_authenticated_actor(
        &UserId::new(owner_id.to_string()),
        &owner_session_hash,
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
    with_authenticated_actor(
        &UserId::new(owner_id.to_string()),
        &owner_session_hash,
        callback_invites.insert(InviteRecord {
            id: format!("inv-{}", "f4".repeat(32)),
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
    let invite_hash_first = invite_hash.clone();
    let invite_hash_second = invite_hash.clone();
    let first = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4, $5)")
            .bind(owner_id)
            .bind(invitation_id)
            .bind(&invite_hash_first)
            .bind("https://issuer.test")
            .bind("auth0|concurrent-member")
            .fetch_one(&app_first)
            .await
    };
    let second = async move {
        sqlx::query_scalar::<_, bool>("SELECT public.claim_invitation($1, $2, $3, $4, $5)")
            .bind(owner_id)
            .bind(invitation_id)
            .bind(&invite_hash_second)
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
            "public.create_invitation(uuid, text, text, text, bigint, uuid)",
            "public.claim_invitation(uuid, uuid, text, text, text)",
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
         'public.claim_invitation(uuid,uuid,text,text,text)'::regprocedure",
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

    let mut guarded = owner.acquire().await?;
    let blocked_rollback = MIGRATOR.undo(&mut *guarded, 38).await.unwrap_err();
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *guarded)
        .await?;
    drop(guarded);
    assert_eq!(migrate_pg_code(&blocked_rollback).as_deref(), Some("55000"));
    sqlx::query("DELETE FROM invitations WHERE id = $1")
        .bind(owner_invitation_id)
        .execute(&owner_actor)
        .await?;
    let tail_delivery: (i32, i32) = sqlx::query_as(
        "SELECT delivered_count, failed_count \
         FROM public.deliver_auth_audit_batch(1000)",
    )
    .fetch_one(&audit)
    .await?;
    assert_eq!(tail_delivery.1, 0, "contract audit delivery must not fail");
    let pending_tail: i64 =
        sqlx::query_scalar("SELECT pending_count FROM public.auth_audit_outbox_stats()")
            .fetch_one(&audit)
            .await?;
    assert_eq!(
        pending_tail, 0,
        "rollback must have no audit obligations left"
    );
    MIGRATOR.undo(owner, 38).await?;
    let function_exists: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('public.claim_invitation(uuid,uuid,text,text,text)') IS NOT NULL",
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
        "version IN (22, 23, 24, 25, 33, 34, 35, 42, 45, 46, 47)",
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
                .contains("successful SQLx migrations 22-25, 33-35, 42, and 45-47 are required")
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
        "worker" => "SET ROLE worker",
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
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_with(opts.options([("app.actor_user_id", user_id.to_string())]))
        .await
        .map_err(Box::<dyn Error>::from)?;
    let identities: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&pool)
        .await?;
    assert_eq!(identities.0, role);
    assert_eq!(
        identities.0, identities.1,
        "actor pools must use the production direct-login identity"
    );
    Ok(pool)
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
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await?;
    let identities: (String, String) = sqlx::query_as("SELECT current_user, session_user")
        .fetch_one(&pool)
        .await?;
    assert_eq!(identities.0, role);
    assert_eq!(
        identities.0, identities.1,
        "role pools must use the production direct-login identity"
    );
    Ok(pool)
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
    let expected_writer_policies = [
        (
            "candidate_fundamental_observations",
            "candidate_source_select_candidate_fundamental_observations",
            "SELECT",
        ),
        (
            "candidate_instrument_registrations",
            "candidate_instrument_registrations_select",
            "SELECT",
        ),
        (
            "candidate_investor_flow_snapshot_rows",
            "candidate_source_select_candidate_investor_flow_snapshot_rows",
            "SELECT",
        ),
        (
            "candidate_investor_flows",
            "candidate_source_select_candidate_investor_flows",
            "SELECT",
        ),
        (
            "candidate_market_status_observations",
            "candidate_source_select_candidate_market_status_observations",
            "SELECT",
        ),
        (
            "candidate_price_instrument_coverage",
            "candidate_source_select_candidate_price_instrument_coverage",
            "SELECT",
        ),
        (
            "candidate_price_instrument_sessions",
            "candidate_source_select_candidate_price_instrument_sessions",
            "SELECT",
        ),
        (
            "candidate_price_publications",
            "candidate_source_select_candidate_price_publications",
            "SELECT",
        ),
        (
            "candidate_price_revalidation_events",
            "candidate_price_revalidation_events_select",
            "SELECT",
        ),
        (
            "candidate_raw_batch_datasets",
            "candidate_source_select_candidate_raw_batch_datasets",
            "SELECT",
        ),
        (
            "candidate_raw_batch_publications",
            "candidate_source_select_candidate_raw_batch_publications",
            "SELECT",
        ),
        (
            "candidate_sector_entries",
            "candidate_source_select_candidate_sector_entries",
            "SELECT",
        ),
        (
            "candidate_sector_versions",
            "candidate_source_select_candidate_sector_versions",
            "SELECT",
        ),
        (
            "candidate_universe_members",
            "candidate_source_select_candidate_universe_members",
            "SELECT",
        ),
        (
            "candidate_universe_registry",
            "candidate_universe_registry_select_research_writer",
            "SELECT",
        ),
        (
            "candidate_universe_snapshots",
            "candidate_source_select_candidate_universe_snapshots",
            "SELECT",
        ),
        (
            "data_batches",
            "data_batches_insert_research_writer",
            "INSERT",
        ),
        (
            "data_batches",
            "data_batches_select_research_writer",
            "SELECT",
        ),
        (
            "dataset_versions",
            "candidate_dataset_versions_select_research_writer",
            "SELECT",
        ),
        (
            "trading_calendar_versions",
            "trading_calendar_versions_insert_research_writer",
            "INSERT",
        ),
        (
            "trading_calendar_versions",
            "trading_calendar_versions_select_research_writer",
            "SELECT",
        ),
        (
            "trading_calendars",
            "trading_calendars_insert_research_writer",
            "INSERT",
        ),
        (
            "trading_calendars",
            "trading_calendars_select_research_writer",
            "SELECT",
        ),
        (
            "trading_calendars",
            "trading_calendars_update_research_writer",
            "UPDATE",
        ),
    ]
    .into_iter()
    .map(|(table, policy, command)| (table.into(), policy.into(), command.into()))
    .collect::<Vec<(String, String, String)>>();
    assert_eq!(
        writer_policies, expected_writer_policies,
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

#[tokio::test]
async fn owner_equity_universe_v2_rls_lifecycle_and_lineage_contract() {
    let super_url = match require_db_url() {
        Ok(url) => url,
        Err(_) => return,
    };
    let (db, owner) = match create_contract_db(&super_url).await {
        Ok(value) => value,
        Err(error) => panic!("setup failed: {error}"),
    };
    let result = owner_equity_universe_v2_contract_body(&super_url, &db, &owner).await;
    let _ = drop_contract_db(&super_url, &db).await;
    if let Err(error) = result {
        panic!("owner equity universe V2 contract FAILED: {error}");
    }
}

async fn owner_equity_universe_v2_contract_body(
    super_url: &str,
    db: &str,
    owner: &PgPool,
) -> Result<(), Box<dyn Error>> {
    const CODE_COMMIT: &str = "3aef74d1a5cdf4368733a8bf45fae66d7de38da7";
    const ENTITLEMENT_REFERENCE: &str = "repo://configs/data-rights/kis.entitlement.json";
    let entitlement_hash = format!("sha256:{}", "a".repeat(64));
    let raw_hash = format!("sha256:{}", "b".repeat(64));
    let artifact_hash = format!("sha256:{}", "c".repeat(64));

    MIGRATOR.run(owner).await?;
    let owner_a: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://owner-equity.test', 'owner-a', 'equity-a@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    let owner_b: Uuid = sqlx::query_scalar(
        "INSERT INTO users (issuer, subject, email) \
         VALUES ('https://owner-equity.test', 'owner-b', 'equity-b@example.test') \
         RETURNING id",
    )
    .fetch_one(owner)
    .await?;
    let owner_a_actor = actor_pool(super_url, db, "migration_owner", &owner_a.to_string()).await?;
    let owner_b_actor = actor_pool(super_url, db, "migration_owner", &owner_b.to_string()).await?;
    for (actor, owner_id) in [(&owner_a_actor, owner_a), (&owner_b_actor, owner_b)] {
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, 'owner')")
            .bind(owner_id)
            .execute(owner)
            .await?;
        let defaults: (i32, i32, i32) = sqlx::query_as(
            "SELECT max_active_instruments, target_observed_sessions, \
                    minimum_observed_sessions \
               FROM owner_equity_universe_policies WHERE owner_user_id = $1",
        )
        .bind(owner_id)
        .fetch_one(actor)
        .await?;
        assert_eq!(defaults, (100, 261, 121));
    }
    for (actor, owner_id) in [(&owner_a_actor, owner_a), (&owner_b_actor, owner_b)] {
        sqlx::query(
            "UPDATE owner_equity_universe_policies \
                SET max_active_instruments = 2, updated_at = clock_timestamp() \
              WHERE owner_user_id = $1",
        )
        .bind(owner_id)
        .execute(actor)
        .await?;
    }

    let app_a = actor_pool(super_url, db, "app", &owner_a.to_string()).await?;
    let app_b = actor_pool(super_url, db, "app", &owner_b.to_string()).await?;
    let membership_a: Uuid = sqlx::query_scalar(
        "INSERT INTO owner_equity_memberships \
         (owner_user_id, instrument_id, transition_actor_user_id, \
          transition_code_commit, transition_entitlement_sha256) \
         VALUES ($1, '005930.KRX', $1, $2, $3) RETURNING id",
    )
    .bind(owner_a)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .fetch_one(&app_a)
    .await?;
    let duplicate = sqlx::query(
        "INSERT INTO owner_equity_memberships \
         (owner_user_id, instrument_id, transition_actor_user_id, \
          transition_code_commit, transition_entitlement_sha256) \
         VALUES ($1, '005930.KRX', $1, $2, $3)",
    )
    .bind(owner_a)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .execute(&app_a)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&duplicate).as_deref(), Some("23505"));
    let _membership_b: Uuid = sqlx::query_scalar(
        "INSERT INTO owner_equity_memberships \
         (owner_user_id, instrument_id, transition_actor_user_id, \
          transition_code_commit, transition_entitlement_sha256) \
         VALUES ($1, '005930.KRX', $1, $2, $3) RETURNING id",
    )
    .bind(owner_b)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .fetch_one(&app_b)
    .await?;
    let app_a_counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM owner_equity_memberships), \
                (SELECT count(*) FROM owner_equity_membership_events)",
    )
    .fetch_one(&app_a)
    .await?;
    assert_eq!(app_a_counts, (1, 1), "owner A cannot read owner B lineage");
    let unscoped_count: i64 = sqlx::query_scalar("SELECT count(*) FROM owner_equity_memberships")
        .fetch_one(owner)
        .await?;
    assert_eq!(unscoped_count, 0, "FORCE RLS fails closed without actor");
    let direct_owner_state_change =
        sqlx::query("UPDATE owner_equity_memberships SET state = 'READY' WHERE id = $1")
            .bind(membership_a)
            .execute(&app_a)
            .await
            .unwrap_err();
    assert_eq!(
        pg_code(&direct_owner_state_change).as_deref(),
        Some("42501")
    );

    let worker = role_pool(super_url, db, "worker").await?;
    let entitlement_id: Uuid = sqlx::query_scalar(
        "INSERT INTO data_entitlements \
         (contract_document_sha256, contract_reference, status, covered_datasets, \
          covered_uses, effective_from, effective_until, managed_by) \
         VALUES ($1, $2, 'ACTIVE', '[\"krx_eod_bars\"]'::jsonb, \
                 '[\"owner_research\"]'::jsonb, DATE '2026-01-01', \
                 DATE '2026-12-31', $3) RETURNING id",
    )
    .bind("a".repeat(64))
    .bind(ENTITLEMENT_REFERENCE)
    .bind(owner_a)
    .fetch_one(owner)
    .await?;
    let calendar_batch_id: Uuid = sqlx::query_scalar(
        "INSERT INTO data_batches \
         (provider, market, batch_date, kind, storage_path, content_sha256, \
          bytes_size, retrieved_at) \
         VALUES ('kis', 'KR', DATE '2026-08-28', 'CALENDAR', \
                 'raw://owner-equity-v2-calendar', $1, 1, \
                 TIMESTAMPTZ '2026-08-28 08:00:00+00') RETURNING id",
    )
    .bind("d".repeat(64))
    .fetch_one(owner)
    .await?;
    sqlx::query(
        "INSERT INTO trading_calendars \
         (exchange, session_date, session_type, timezone, source, source_version, \
          source_batch_id, content_sha256, retrieved_at) \
         VALUES ('KRX', DATE '2026-08-28', 'TRADING', 'Asia/Seoul', 'kis', 'test', \
                 $1, $2, TIMESTAMPTZ '2026-08-28 08:00:00+00')",
    )
    .bind(calendar_batch_id)
    .bind("d".repeat(64))
    .execute(owner)
    .await?;
    let illegal_transition = sqlx::query(
        "UPDATE owner_equity_memberships \
         SET state = 'BACKFILLING', transition_actor_user_id = $2, \
             transition_code_commit = $3, transition_entitlement_sha256 = $4, \
             updated_at = now() WHERE id = $1",
    )
    .bind(membership_a)
    .bind(owner_a)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&illegal_transition).as_deref(), Some("23514"));
    for state in ["VALIDATING", "BACKFILLING", "MATERIALIZING"] {
        sqlx::query(
            "UPDATE owner_equity_memberships \
             SET state = $2, transition_actor_user_id = $3, \
                 transition_code_commit = $4, transition_entitlement_sha256 = $5, \
                 updated_at = now() WHERE id = $1",
        )
        .bind(membership_a)
        .bind(state)
        .bind(owner_a)
        .bind(CODE_COMMIT)
        .bind(&entitlement_hash)
        .execute(&worker)
        .await?;
    }

    let skipped_generation = sqlx::query(
        "INSERT INTO owner_equity_instrument_generations \
         (membership_id, owner_user_id, instrument_id, generation, \
          target_observed_sessions, minimum_observed_sessions, observed_sessions, \
          first_session, last_session) \
         VALUES ($1, $2, '005930.KRX', 2, 261, 121, 121, \
                 '2026-01-02', '2026-06-30')",
    )
    .bind(membership_a)
    .bind(owner_a)
    .execute(&worker)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&skipped_generation).as_deref(), Some("23514"));
    let generation_id: Uuid = sqlx::query_scalar(
        "INSERT INTO owner_equity_instrument_generations \
         (membership_id, owner_user_id, instrument_id, generation, \
          target_observed_sessions, minimum_observed_sessions, observed_sessions, \
          first_session, last_session) \
         VALUES ($1, $2, '005930.KRX', 1, 261, 121, 121, \
                 '2026-01-02', '2026-06-30') RETURNING id",
    )
    .bind(membership_a)
    .bind(owner_a)
    .fetch_one(&worker)
    .await?;
    sqlx::query(
        "INSERT INTO owner_equity_generation_admissions \
         (generation_id, owner_user_id, membership_id, instrument_id, generation, \
          raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256, \
          capture_code_commit, materializer_code_commit) \
         VALUES ($1, $2, $3, '005930.KRX', 1, $4, $5, $6, $7, $7)",
    )
    .bind(generation_id)
    .bind(owner_a)
    .bind(membership_a)
    .bind(&raw_hash)
    .bind(&artifact_hash)
    .bind(&entitlement_hash)
    .bind(CODE_COMMIT)
    .execute(&worker)
    .await?;
    sqlx::query(
        "UPDATE owner_equity_memberships \
         SET state = 'READY', transition_actor_user_id = $2, \
             transition_code_commit = $3, transition_entitlement_sha256 = $4, \
             updated_at = now() WHERE id = $1",
    )
    .bind(membership_a)
    .bind(owner_a)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .execute(&worker)
    .await?;

    let scheduled: (Uuid, bool) = sqlx::query_as(
        "SELECT job_id, inserted FROM schedule_owner_equity_incremental(\
            $1, $2, DATE '2026-08-28', $3, $4, $5)",
    )
    .bind(owner_a)
    .bind(membership_a)
    .bind(CODE_COMMIT)
    .bind(ENTITLEMENT_REFERENCE)
    .bind(&entitlement_hash)
    .fetch_one(&worker)
    .await?;
    assert!(scheduled.1);
    let scheduled_replay: (Uuid, bool) = sqlx::query_as(
        "SELECT job_id, inserted FROM schedule_owner_equity_incremental(\
            $1, $2, DATE '2026-08-28', $3, $4, $5)",
    )
    .bind(owner_a)
    .bind(membership_a)
    .bind(CODE_COMMIT)
    .bind(ENTITLEMENT_REFERENCE)
    .bind(&entitlement_hash)
    .fetch_optional(&worker)
    .await?
    .unwrap_or((scheduled.0, false));
    assert_eq!(scheduled_replay, (scheduled.0, false));
    let incremental_payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload_json FROM jobs WHERE id = $1")
            .bind(scheduled.0)
            .fetch_one(&worker)
            .await?;
    assert_eq!(incremental_payload["action"], "INCREMENTAL");
    assert_eq!(incremental_payload["expected_generation"], 2);

    let universe_hash: String = sqlx::query_scalar(
        "SELECT 'sha256:' || encode(sha256(convert_to('005930.KRX', 'UTF8')), 'hex')",
    )
    .fetch_one(owner)
    .await?;
    let snapshot_id: Uuid = sqlx::query_scalar(
        "INSERT INTO owner_equity_signal_snapshots \
         (owner_user_id, as_of_session, universe_sha256, row_count, signal_code_commit) \
         VALUES ($1, '2026-08-31', $2, 1, $3) RETURNING id",
    )
    .bind(owner_a)
    .bind(&universe_hash)
    .bind(CODE_COMMIT)
    .fetch_one(&worker)
    .await?;
    sqlx::query(
        "INSERT INTO owner_equity_signal_snapshot_rows \
         (snapshot_id, owner_user_id, instrument_id, membership_id, \
          generation_id, generation, rank, signals_json) \
         VALUES ($1, $2, '005930.KRX', $3, $4, 1, 1, '{\"return_120\":\"0.1\"}')",
    )
    .bind(snapshot_id)
    .bind(owner_a)
    .bind(membership_a)
    .bind(generation_id)
    .execute(&worker)
    .await?;
    let draft_visibility: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM owner_equity_signal_snapshots), \
                (SELECT count(*) FROM owner_equity_signal_snapshot_rows)",
    )
    .fetch_one(&app_a)
    .await?;
    assert_eq!(
        draft_visibility,
        (0, 0),
        "app RLS must hide unpublished signal headers and rows"
    );
    sqlx::query("UPDATE owner_equity_signal_snapshots SET published_at = now() WHERE id = $1")
        .bind(snapshot_id)
        .execute(&worker)
        .await?;
    let immutable_admission = sqlx::query(
        "UPDATE owner_equity_generation_admissions \
         SET artifact_manifest_sha256 = $2 WHERE generation_id = $1",
    )
    .bind(generation_id)
    .bind(&raw_hash)
    .execute(&owner_a_actor)
    .await
    .unwrap_err();
    assert_eq!(pg_code(&immutable_admission).as_deref(), Some("42501"));

    sqlx::query("SELECT disable_owner_equity_membership($1, $2, $3)")
        .bind(membership_a)
        .bind(CODE_COMMIT)
        .bind(&entitlement_hash)
        .execute(&app_a)
        .await?;
    let replacement: Uuid = sqlx::query_scalar(
        "INSERT INTO owner_equity_memberships \
         (owner_user_id, instrument_id, transition_actor_user_id, \
          transition_code_commit, transition_entitlement_sha256) \
         VALUES ($1, '005930.KRX', $1, $2, $3) RETURNING id",
    )
    .bind(owner_a)
    .bind(CODE_COMMIT)
    .bind(&entitlement_hash)
    .fetch_one(&app_a)
    .await?;
    assert_ne!(
        replacement, membership_a,
        "disable is soft and re-add is new lineage"
    );
    let hard_delete = sqlx::query("DELETE FROM owner_equity_memberships WHERE id = $1")
        .bind(membership_a)
        .execute(&owner_a_actor)
        .await
        .unwrap_err();
    assert_eq!(pg_code(&hard_delete).as_deref(), Some("42501"));

    let rollback = MIGRATOR.undo(owner, 1).await.unwrap_err();
    assert_eq!(migrate_pg_code(&rollback).as_deref(), Some("55000"));
    assert_eq!(applied_count(owner).await?, up_migration_count() as i64);
    let _ = (entitlement_id, calendar_batch_id);
    Ok(())
}
