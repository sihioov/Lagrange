//! Actual-role maintenance SQL against the existing fully migrated scratch DB.
//! Fault injection is supervisor-only fixture construction, never a production
//! repair path. Synthetic admissions do not prove any filesystem/source input.

mod common;
mod owner_beta_daily_support;

use std::error::Error;

use job_queue::owner_beta::runner::{
    daily_runtime::DailyProcessMode, daily_schedule::schedule_daily_once,
};
use owner_beta_daily_support::{DailyFixture, run};
use serde_json::Value;
use sqlx::{ConnectOptions, Connection, Row, postgres::PgConnectOptions};

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
const AUDIT: &str = include_str!("../../../deploy/db/owner-beta-daily-backup-audit.sql");
const REPAIR: &str = include_str!("../../../deploy/db/owner-beta-daily-reservation-restore.sql");

async fn execute(
    f: &DailyFixture,
    sql: &'static str,
    role: &str,
    impersonate: bool,
) -> Result<Vec<String>, sqlx::Error> {
    let options: PgConnectOptions = f.db.role_url(role).parse()?;
    let mut connection =
        sqlx::PgConnection::connect_with(&options.disable_statement_logging()).await?;
    let result = async {
        let identity: (String, String) = sqlx::query_as("SELECT current_user,session_user")
            .fetch_one(&mut connection)
            .await?;
        assert_eq!(identity, (role.to_owned(), role.to_owned()));
        if impersonate {
            sqlx::raw_sql("SET ROLE migration_owner")
                .execute(&mut connection)
                .await?;
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_all(&mut connection)
            .await?
            .iter()
            .map(|row| row.try_get("daily_backup_result"))
            .collect()
    }
    .await;
    // In particular close failed transactions before checking state/cleanup.
    connection.close().await?;
    result
}

fn denied(result: Result<Vec<String>, sqlx::Error>, code: &str) {
    let error = result.expect_err("maintenance unexpectedly succeeded");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some(code),
        "maintenance SQLSTATE differs"
    );
}

async fn audit(f: &DailyFixture, role: &str) -> TestResult<Value> {
    let rows = execute(f, AUDIT, role, false).await?;
    assert_eq!(rows.len(), 1);
    Ok(serde_json::from_str(&rows[0])?)
}

async fn repair(f: &DailyFixture) -> TestResult {
    assert_eq!(
        execute(f, REPAIR, "migration_owner", false).await?,
        vec!["DAILY_BACKUP_RESERVATIONS: VERIFIED"]
    );
    Ok(())
}

async fn seeded(f: &DailyFixture, second: bool) -> TestResult {
    f.ready().await?;
    if second {
        sqlx::query("INSERT INTO user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json,daily_enabled) VALUES($1,'daily_fixture','1.0.0','{\"second\":true}',true)")
            .bind(f.owner_id).execute(&f.app).await?;
    }
    schedule_daily_once(&f.worker, DailyProcessMode::OwnerOnly).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM owner_beta_daily_reservations")
        .fetch_one(&f.db.pool)
        .await?;
    assert_eq!(count, if second { 2 } else { 1 });
    sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
        .execute(&f.db.pool)
        .await?;
    Ok(())
}

async fn history(f: &DailyFixture) -> TestResult<Value> {
    // Supervisor observation of every row, independent of serving-role RLS.
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object(\
         'jobs',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM jobs t),\
         'attempts',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM job_attempts t),\
         'runs',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM owner_beta_recommendation_runs t),\
         'items',(SELECT jsonb_agg(to_jsonb(t) ORDER BY recommendation_run_id,instrument_id) FROM owner_beta_recommendation_items t),\
         'control',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM owner_beta_daily_control t),\
         'admissions',(SELECT jsonb_agg(to_jsonb(t) ORDER BY admission_sha256) FROM owner_beta_daily_admissions t))",
    ).fetch_one(&f.db.pool).await?)
}

async fn reservations(f: &DailyFixture, omit_repair_time: bool) -> TestResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(CASE WHEN $1 THEN to_jsonb(r)-'created_at' ELSE to_jsonb(r) END ORDER BY run_id),'[]'::jsonb) FROM owner_beta_daily_reservations r",
    ).bind(omit_repair_time).fetch_one(&f.db.pool).await?)
}

async fn lose_reservations(f: &DailyFixture) -> TestResult {
    // Model a damaged restored database. The maintenance script never has a
    // trigger/replication bypass and must repair through the original guards.
    sqlx::raw_sql("BEGIN; SET LOCAL session_replication_role=replica; DELETE FROM owner_beta_daily_reservations; COMMIT;")
        .execute(&f.db.pool).await?;
    Ok(())
}

#[tokio::test]
async fn daily_backup_audit_checks_real_history_without_mutation_or_rls_blindness() {
    run("backup_audit_real_history", |f| async move {
        seeded(&f, false).await?;
        // A second identity exposes an accidental repeated superuser scan.
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES(gen_random_uuid(),'https://backup.invalid','other','other@backup.invalid')")
            .execute(&f.db.pool).await?;
        let before = history(&f).await?;
        let saved = reservations(&f, false).await?;
        let payloads = f.job_payloads().await?;
        let actual = audit(&f, "migration_owner").await?;
        assert_eq!(audit(&f, "postgres").await?, actual);
        assert_eq!(actual["schema"], "lagrange-daily-backup-db-v1");
        assert_eq!(actual["control"]["enabled"], false);
        assert_eq!(actual["run_count"], 1);
        assert_eq!(actual["tables"].as_array().ok_or("tables")?.len(), 14);
        assert_eq!(actual["admissions"].as_array().ok_or("admissions")?.len(), 1);
        assert_eq!(history(&f).await?, before);
        assert_eq!(reservations(&f, false).await?, saved);
        assert_eq!(f.job_payloads().await?, payloads);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_identity_visibility_rejects_effective_rls_without_policy_bypass() {
    run("backup_identity_visibility", |f| async move {
        seeded(&f, false).await?;
        let before = history(&f).await?;
        let saved = reservations(&f, false).await?;
        let options: PgConnectOptions = f.db.role_url("migration_owner").parse()?;
        let mut connection =
            sqlx::PgConnection::connect_with(&options.disable_statement_logging()).await?;
        let identity: (String, String) = sqlx::query_as("SELECT current_user,session_user")
            .fetch_one(&mut connection)
            .await?;
        assert_eq!(identity, ("migration_owner".to_owned(), "migration_owner".to_owned()));
        let catalog: (bool, bool, String) = sqlx::query_as(
            "SELECT relrowsecurity,relforcerowsecurity,pg_get_userbyid(relowner)::text FROM pg_class WHERE oid='public.users'::regclass",
        ).fetch_one(&mut connection).await?;
        assert_eq!(catalog, (true, false, "migration_owner".to_owned()));
        let filtering: bool = sqlx::query_scalar(
            "SELECT pg_catalog.row_security_active('public.users'::regclass)",
        ).fetch_one(&mut connection).await?;
        assert!(!filtering);
        assert_eq!(audit(&f, "migration_owner").await?["run_count"], 1);
        repair(&f).await?;

        // Disposable schema fault: FORCE RLS makes this actual login unable
        // to enumerate identities. Neither maintenance path may hide that
        // loss or turn it into a successful empty-history audit/repair.
        sqlx::raw_sql("ALTER TABLE public.users FORCE ROW LEVEL SECURITY")
            .execute(&f.db.pool)
            .await?;
        let filtering: bool = sqlx::query_scalar(
            "SELECT pg_catalog.row_security_active('public.users'::regclass)",
        ).fetch_one(&mut connection).await?;
        assert!(filtering);
        let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM public.users")
            .fetch_one(&mut connection)
            .await?;
        assert_eq!(visible, 0);
        connection.close().await?;
        for (sql, code) in [
            (AUDIT, "DAILY_BACKUP_IDENTITY_VISIBILITY"),
            (REPAIR, "DAILY_RESTORE_IDENTITY_VISIBILITY"),
        ] {
            let error = execute(&f, sql, "migration_owner", false)
                .await
                .expect_err("filtered identities must reject maintenance");
            let database = error.as_database_error().ok_or("expected database error")?;
            assert_eq!(database.code().as_deref(), Some("23514"));
            assert!(database.message() == code, "wrong fixed visibility rejection");
        }
        assert_eq!(history(&f).await?, before);
        assert_eq!(reservations(&f, false).await?, saved);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_audit_rejects_missing_control_and_enabled_capture() {
    run("backup_audit_control", |f| async move {
        denied(execute(&f, AUDIT, "migration_owner", false).await, "23514");
        f.control(true).await?;
        denied(execute(&f, AUDIT, "migration_owner", false).await, "23514");
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&f.db.pool)
            .await?;
        assert_eq!(audit(&f, "migration_owner").await?["run_count"], 0);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_backup_audit_detects_poll_state_change_without_reading_provider_bodies() {
    run("backup_audit_fingerprint", |f| async move {
        seeded(&f, false).await?;
        let before = audit(&f, "migration_owner").await?;
        f.next_schedule().await?;
        let after = audit(&f, "migration_owner").await?;
        assert_eq!(before["admissions"], after["admissions"]);
        assert_ne!(before["tables"], after["tables"]);
        assert!(!after.to_string().contains("entitlement_reference"));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_backup_restore_recovers_original_reservation_once_without_new_job() {
    run("backup_restore_original", |f| async move {
        seeded(&f, false).await?;
        let expected = reservations(&f, true).await?;
        let before = history(&f).await?;
        lose_reservations(&f).await?;
        denied(execute(&f, AUDIT, "migration_owner", false).await, "23514");
        repair(&f).await?;
        assert_eq!(reservations(&f, true).await?, expected);
        assert_eq!(history(&f).await?, before);
        let first = reservations(&f, false).await?;
        repair(&f).await?;
        assert_eq!(reservations(&f, false).await?, first);
        assert_eq!(audit(&f, "migration_owner").await?["run_count"], 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_backup_maintenance_rejects_serving_roles_and_impersonated_login() {
    run("backup_restore_roles", |f| async move {
        seeded(&f, false).await?;
        let before = history(&f).await?;
        for sql in [AUDIT, REPAIR] {
            for role in [
                "app",
                "worker",
                "research_writer",
                "daily_input_admitter",
                "admin",
                "audit_writer",
            ] {
                denied(execute(&f, sql, role, false).await, "42501");
            }
            denied(execute(&f, sql, "postgres", true).await, "42501");
        }
        denied(execute(&f, REPAIR, "postgres", false).await, "42501");
        assert_eq!(history(&f).await?, before);
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_backup_restore_requires_disabled_control_and_keeps_empty_history_empty() {
    run("backup_restore_disabled", |f| async move {
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        f.control(true).await?;
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&f.db.pool)
            .await?;
        let before = history(&f).await?;
        repair(&f).await?;
        assert_eq!(history(&f).await?, before);
        assert_eq!(reservations(&f, false).await?, serde_json::json!([]));
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn daily_backup_restore_never_reseals_old_config_or_activation_epochs() {
    run("backup_restore_sealed_epochs", |f| async move {
        seeded(&f, false).await?;
        let expected = reservations(&f, true).await?;
        lose_reservations(&f).await?;
        sqlx::query("UPDATE user_strategy_configs SET config_json='{\"changed\":true}',daily_enabled=false WHERE owner_user_id=$1")
            .bind(f.owner_id).execute(&f.app).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=true").execute(&f.db.pool).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false").execute(&f.db.pool).await?;
        let before = history(&f).await?;
        repair(&f).await?;
        assert_eq!(reservations(&f, true).await?, expected);
        assert_eq!(history(&f).await?, before);
        let stale: bool = sqlx::query_scalar("SELECT r.config_epoch<>s.daily_epoch AND r.activation_epoch<>c.activation_epoch FROM owner_beta_daily_reservations r JOIN user_strategy_configs s ON s.id=r.strategy_config_id CROSS JOIN owner_beta_daily_control c")
            .fetch_one(&f.db.pool).await?;
        assert!(stale);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_restore_rejects_replacement_key_without_inserting_reservation() {
    run("backup_restore_replacement_key", |f| async move {
        seeded(&f, false).await?;
        lose_reservations(&f).await?;
        sqlx::raw_sql("BEGIN; SET LOCAL session_replication_role=replica; UPDATE jobs SET idempotency_key='replacement-key'; COMMIT;")
            .execute(&f.db.pool).await?;
        let before = history(&f).await?;
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        assert_eq!(reservations(&f, false).await?, serde_json::json!([]));
        assert_eq!(history(&f).await?, before);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_restore_rejects_conflicting_existing_date_without_overwrite() {
    run("backup_restore_conflicting_date", |f| async move {
        seeded(&f, false).await?;
        sqlx::raw_sql("BEGIN; SET LOCAL session_replication_role=replica; UPDATE owner_beta_daily_reservations SET session_date=session_date+1; COMMIT;")
            .execute(&f.db.pool).await?;
        let before = reservations(&f, false).await?;
        denied(execute(&f, AUDIT, "migration_owner", false).await, "23514");
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        assert_eq!(reservations(&f, false).await?, before);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_restore_rolls_back_earlier_repair_when_later_run_is_invalid() {
    run("backup_restore_atomic", |f| async move {
        seeded(&f, true).await?;
        lose_reservations(&f).await?;
        // ORDER BY id makes one legitimate repair precede the damaged run.
        sqlx::raw_sql("BEGIN; SET LOCAL session_replication_role=replica; UPDATE jobs SET idempotency_key='damaged-last' WHERE id=(SELECT job_id FROM owner_beta_recommendation_runs ORDER BY id DESC LIMIT 1); COMMIT;")
            .execute(&f.db.pool).await?;
        let before = history(&f).await?;
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        assert_eq!(reservations(&f, false).await?, serde_json::json!([]));
        assert_eq!(history(&f).await?, before);
        Ok(())
    }).await;
}

#[tokio::test]
async fn daily_backup_orphan_job_is_blocked_without_replacement_run() {
    run("backup_restore_orphan_job", |f| async move {
        seeded(&f, false).await?;
        lose_reservations(&f).await?;
        sqlx::raw_sql("BEGIN; SET LOCAL session_replication_role=replica; DELETE FROM owner_beta_recommendation_runs; COMMIT;")
            .execute(&f.db.pool).await?;
        let before = history(&f).await?;
        denied(execute(&f, AUDIT, "migration_owner", false).await, "23514");
        denied(execute(&f, REPAIR, "migration_owner", false).await, "23514");
        assert_eq!(history(&f).await?, before);
        Ok(())
    }).await;
}
