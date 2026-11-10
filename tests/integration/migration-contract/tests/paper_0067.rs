use super::{Daily0060Db, Daily0060Result, MIGRATOR, admin_pool};
use serde_json::{Value, json};
use sqlx::Acquire;
use sqlx::postgres::PgPool;
use sqlx::types::Uuid;
use std::env;

#[path = "paper_0067/audit.rs"]
mod audit;

struct FixtureClock {
    admin: PgPool,
    nonce: String,
}
impl FixtureClock {
    async fn new() -> Daily0060Result<Self> {
        let url = require_r5_03_database_url()?;
        let admin = admin_pool(&url)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        verify_r5_03_postgres_identity(&admin).await?;
        Ok(Self {
            admin,
            nonce: env::var("R5_03_ISOLATED_NONCE")?,
        })
    }
    async fn set(&self, seconds: i64) -> Daily0060Result {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        // Four session-day advances plus the initial after-cutoff shift are
        // needed for retained recovery and three fresh post-OFF SQL sessions.
        // This bounded offset is process-only in the identity-checked fixture.
        if !(0..=432000).contains(&seconds)
            || env::var("R5_03_ISOLATED_NONCE")? != self.nonce
            || env::var("R5_03_FIXTURE_CLOCK_FILE")? != "/tmp/r5-03-clock/offset"
        {
            return Err("isolated fixture clock context changed".into());
        }
        verify_r5_03_postgres_identity(&self.admin).await?;
        let path = std::path::Path::new("/tmp/r5-03-clock/offset");
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.uid() != 999 || meta.nlink() != 1 {
            return Err("isolated fixture clock file identity".into());
        }
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)?
            .write_all(format!("+{seconds}\n").as_bytes())?;
        // Check actual guest PG/test realtime; no SQL function/GUC time seam.
        for _ in 0..40 {
            let pg: i64 =
                sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                    .fetch_one(&self.admin)
                    .await?;
            let rust = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs() as i64;
            if (pg - rust).abs() <= 2 {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err("guest PostgreSQL/test realtime clocks diverged".into())
    }
    async fn after_cutoff(&self) -> Daily0060Result<i64> {
        let shift:i64=sqlx::query_scalar("SELECT CASE WHEN (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::time<time '16:30' THEN ceil(extract(epoch FROM (date_trunc('day',clock_timestamp() AT TIME ZONE 'Asia/Seoul')+interval '17 hours'-(clock_timestamp() AT TIME ZONE 'Asia/Seoul'))))::bigint ELSE 0::bigint END").fetch_one(&self.admin).await?;
        self.set(shift).await?;
        Ok(shift)
    }
}
impl Drop for FixtureClock {
    fn drop(&mut self) {
        let _ = std::fs::write("/tmp/r5-03-clock/offset", b"+0\n");
    }
}

#[derive(Clone)]
struct SqlWitness {
    session: String,
    run: Uuid,
    admission: String,
    root: String,
    source: String,
}
struct SqlPaper {
    account: Uuid,
    config: Uuid,
    cohort: Value,
    digest: String,
    witness: SqlWitness,
}

async fn synthetic_digest(db: &Daily0060Db, label: &str) -> Daily0060Result<String> {
    Ok(
        sqlx::query_scalar("SELECT daily_0060_digest('SYNTHETIC-SQL-ONLY:'||$1::text)")
            .bind(label)
            .fetch_one(&db.owner)
            .await?,
    )
}

const PAPER_0067_UP: &str =
    include_str!("../../../../migrations/0067_owner_beta_paper_ledger_v1.up.sql");
const PAPER_0068_UP: &str =
    include_str!("../../../../migrations/0068_owner_beta_paper_operations_v1.up.sql");

fn emit_r5_paper_test_witness(
    stage: &str,
    outcome: &str,
    sqlstate: Option<&str>,
    backend_pid: Option<i32>,
    blocker_pid: Option<i32>,
    advisory_blocked: Option<bool>,
    completed_before_unlock: Option<bool>,
) {
    const STAGES: &[&str] = &[
        "READY_SETUP",
        "READY_T1_TARGET",
        "READY_T1_SETTLE",
        "READY_T2_PREVIEW",
        "READY_T2_WAIT",
        "READY_T1_VALUE",
        "READY_TRIAL_APPLY",
        "READY_TRIAL_READBACK",
        "READY_REVOKE_RETRY",
        "APPLY_POOL_ACQUIRE",
        "APPLY_QUERY",
        "TERMINAL_UPDATE",
        "RESET_WAIT",
    ];
    const OUTCOMES: &[&str] = &[
        "ENTERED",
        "POOL_PENDING",
        "QUERY_PENDING",
        "CLIENT_TIMEOUT",
        "UPDATE_OK",
        "DATABASE_ERROR",
        "NON_DATABASE_ERROR",
    ];
    assert!(STAGES.contains(&stage));
    assert!(OUTCOMES.contains(&outcome));
    if let Some(code) = sqlstate {
        assert!(
            code.len() == 5
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        );
    }
    for pid in [backend_pid, blocker_pid].into_iter().flatten() {
        assert!(pid > 0);
    }
    let mut witness = serde_json::Map::new();
    witness.insert("stage".into(), json!(stage));
    witness.insert("outcome".into(), json!(outcome));
    if let Some(value) = sqlstate {
        witness.insert("sqlstate".into(), json!(value));
    }
    if let Some(value) = backend_pid {
        witness.insert("backend_pid".into(), json!(value));
    }
    if let Some(value) = blocker_pid {
        witness.insert("blocker_pid".into(), json!(value));
    }
    if let Some(value) = advisory_blocked {
        witness.insert("advisory_blocked".into(), json!(value));
    }
    if let Some(value) = completed_before_unlock {
        witness.insert("completed_before_unlock".into(), json!(value));
    }
    eprintln!(
        "R5_PAPER_TEST_WITNESS:{}",
        serde_json::Value::Object(witness)
    );
}

// These diagnostic identifiers name exact source-owned RAISE sites only.
const R5_PAPER_SQL_GUARDS: &[&str] = &[
    "paper_audit_account_account_strategy_bindings",
    "paper_audit_account_cash_ledger",
    "paper_audit_account_daily_equity",
    "paper_audit_account_fills",
    "paper_audit_account_orders",
    "paper_audit_account_owner_beta_paper_control",
    "paper_audit_account_owner_beta_paper_events",
    "paper_audit_account_owner_beta_paper_lineage",
    "paper_audit_account_owner_beta_paper_sessions",
    "paper_audit_account_paper_rebalance_previews",
    "paper_audit_account_pending_targets",
    "paper_audit_account_positions",
    "paper_audit_daily_admission_pin",
    "paper_audit_daily_attempt_sequence",
    "paper_audit_daily_cancel",
    "paper_audit_daily_failed",
    "paper_audit_daily_final_state",
    "paper_audit_daily_premature_output",
    "paper_audit_daily_queued",
    "paper_audit_daily_reservation",
    "paper_audit_daily_running",
    "paper_audit_daily_success",
    "paper_audit_delivery",
    "paper_audit_fill",
    "paper_audit_history_orphan",
    "paper_audit_lineage",
    "paper_audit_lineage_orphan",
    "paper_audit_notification",
    "paper_audit_obligation",
    "paper_audit_outbox",
    "paper_audit_publication_orphan",
    "paper_audit_receipt",
    "paper_audit_reservation",
    "paper_audit_session",
    "paper_audit_settlement_rows",
    "paper_audit_source_chain",
    "paper_audit_source_orphan",
    "paper_audit_valuation_rows",
    "paper_recovery_v2_control",
    "paper_recovery_v2_rights",
];

// Only closed stage names, validated SQLSTATEs and numeric cursor positions
// leave a failed positive fixture. Never format the underlying database error.
fn r5_sql_stage<T, E>(stage: &'static str, result: Result<T, E>) -> Daily0060Result<T>
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    assert!(matches!(
        stage,
        "RESET_SETUP"
            | "RESET_BARRIER"
            | "RESET_WAIT"
            | "RESET_BEGIN"
            | "RESET_RECONCILE"
            | "RESET_OFF"
            | "RESET_ENABLE"
            | "RESET_READBACK"
            | "AUDIT_READ"
            | "AUDIT_SETUP"
            | "AUDIT_TARGET"
            | "AUDIT_E_SOURCE"
            | "AUDIT_SETTLE"
            | "AUDIT_VALUE"
            | "AUDIT_UPGRADE"
            | "RECOVERY_V1_SETUP"
            | "RECOVERY_V1_TARGET"
            | "RECOVERY_V1_E_SOURCE"
            | "RECOVERY_V1_SETTLE"
            | "RECOVERY_V1_BUILD"
            | "RECOVERY_V1_TRANSITION"
            | "RECOVERY_V1_READBACK"
            | "RECOVERY_V2_SETUP"
            | "RECOVERY_V2_TARGET"
            | "RECOVERY_V2_E_SOURCE"
            | "RECOVERY_V2_SETTLE"
            | "RECOVERY_V2_BUILD"
            | "RECOVERY_V2_TRANSITION"
            | "RECOVERY_V2_READBACK"
    ));
    result.map_err(|error| {
        let error = error.into();
        let sql_error = error.downcast_ref::<sqlx::Error>().or_else(|| {
            match error.downcast_ref::<sqlx::migrate::MigrateError>() {
                Some(sqlx::migrate::MigrateError::Execute(e))
                | Some(sqlx::migrate::MigrateError::ExecuteMigration(e, _)) => Some(e),
                _ => None,
            }
        });
        let database = sql_error.and_then(sqlx::Error::as_database_error);
        let code = database.and_then(|e| e.code()).filter(|code| {
            code.len() == 5
                && code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        });
        let mut witness = json!({"stage":stage,"outcome":if database.is_some() {
            "DATABASE_ERROR"
        } else {
            "NON_DATABASE_ERROR"
        }});
        if let Some(code) = &code {
            witness["sqlstate"] = json!(code.as_ref());
        }
        let mut guard_id = None;
        if let Some(pg) =
            database.and_then(|e| e.try_downcast_ref::<sqlx::postgres::PgDatabaseError>())
        {
            // Return the matched literal from our finite list, never an
            // arbitrary server identifier or an error message prefix.
            guard_id = R5_PAPER_SQL_GUARDS
                .iter()
                .copied()
                .find(|name| pg.constraint() == Some(*name));
            if let Some(name) = guard_id {
                witness["guard_id"] = json!(name);
            }
            let cursor = match pg.position() {
                Some(sqlx::postgres::PgErrorPosition::Original(position)) => {
                    Some(("ORIGINAL", position))
                }
                Some(sqlx::postgres::PgErrorPosition::Internal { position, .. }) => {
                    Some(("INTERNAL", position))
                }
                None => None,
            };
            if let Some((kind, position)) = cursor.filter(|(_, p)| (1..=1_000_000).contains(p)) {
                witness["sql_position_kind"] = json!(kind);
                witness["sql_position"] = json!(position);
            }
        }
        eprintln!("R5_PAPER_TEST_WITNESS:{witness}");
        // libtest formats Result errors with Debug. Return fixed metadata too,
        // so it cannot persist PgDatabaseError.message/detail/where/query.
        format!(
            "R5_PAPER_STAGE_FAILED:{stage} SQLSTATE={} GUARD={}",
            code.as_deref().unwrap_or("UNKNOWN"),
            guard_id.unwrap_or("UNKNOWN")
        )
        .into()
    })
}

async fn seed_witness(
    db: &Daily0060Db,
    config: Uuid,
    cohort: &Value,
    session: &str,
    parent: Option<&str>,
    weight: bool,
) -> Daily0060Result<SqlWitness> {
    // SYNTHETIC relational predecessor data, never genuine Raw/sealed evidence.
    // The accepted bundle encoder supplies JSON shape. Only predecessor USER
    // triggers are suspended during trusted fixture insertion; FK/CHECK/RLS
    // and every 0067 guard remain active. Restore all triggers before serving
    // calls. Observation timestamps use actual now, never backdated lineage.
    let (mut input, _, _) = super::daily_0060_bundle(db, session).await?;
    let admission = synthetic_digest(db, &format!("admission:{session}")).await?;
    input["admission_sha256"] = json!(admission);
    input["manifest"]["parent_admission_sha256"] = json!(parent);
    input["admission"]["parent_admission_sha256"] = json!(parent);
    let slot = synthetic_digest(db, &format!("slot:{session}")).await?;
    let root = synthetic_digest(db, &format!("root:{session}")).await?;
    let source = synthetic_digest(db, &format!("source:{session}")).await?;
    let original = synthetic_digest(db, &format!("originals:{session}")).await?;
    let target = synthetic_digest(db, &format!("W:{session}")).await?;
    let (run, job, attempt) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut tx = db.owner.begin().await?;
    let tables = [
        "jobs",
        "job_attempts",
        "owner_beta_recommendation_runs",
        "owner_beta_recommendation_items",
        "owner_beta_daily_reservations",
        "owner_beta_daily_publication_claims",
    ];
    let trigger_sql = "SELECT jsonb_agg(jsonb_build_array(c.relname,t.tgname,t.tgenabled) ORDER BY c.relname,t.tgname) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE NOT t.tgisinternal AND c.relname=ANY($1)";
    let before: Value = sqlx::query_scalar(trigger_sql)
        .bind(&tables[..])
        .fetch_one(&mut *tx)
        .await?;
    for table in tables {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE public.{table} DISABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await?;
    }
    let now: String = sqlx::query_scalar("SELECT clock_timestamp()::text")
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO owner_beta_daily_admissions(admission_sha256,owner_user_id,policy_sha256,content_sha256,manifest_sha256,parent_admission_sha256,range_start,range_end,session_count,bar_count,evidence_json) VALUES($1,$2,$3,$4,$5,$6,'2016-08-29',$7::text::date,2453,26983,$8)").bind(&admission).bind(db.owner_id).bind(hash('b')).bind(hash('d')).bind(hash('e')).bind(parent).bind(session).bind(&input).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO jobs(id,owner_user_id,job_type,status,payload_json,max_attempts,attempt_count,started_at,finished_at) VALUES($1,$2,'owner_beta_price_recommendation','SUCCEEDED','{}',3,1,$3::text::timestamptz,$3::text::timestamptz)").bind(job).bind(db.owner_id).bind(&now).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO job_attempts(id,job_id,attempt_no,outcome,claimed_by,started_at,finished_at) VALUES($1,$2,1,'SUCCEEDED','synthetic-source',$3::text::timestamptz,$3::text::timestamptz)").bind(attempt).bind(job).bind(&now).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_recommendation_runs(id,owner_user_id,strategy_config_id,job_id,as_of,status,input_kind,trigger_kind,strategy_id,strategy_version,strategy_config_json,strategy_config_sha256,daily_pins,daily_config_epoch,daily_activation_epoch,daily_trigger_version,factor_snapshot_sha256,target_snapshot_sha256,cash_weight,started_at,finished_at,updated_at) SELECT $1,$2,s.id,$3,$4::text::date,'SUCCEEDED','owner_beta_daily_etf11_price_only_v1','DAILY',s.strategy_id,s.strategy_version,s.config_json,$5,jsonb_build_object('content_sha256',$6::text,'manifest_sha256',$7::text,'admission_sha256',$8::text,'policy_sha256',$9::text),s.daily_epoch,($10::jsonb->>'activation_epoch')::bigint,'owner-beta-daily-trigger-v1',$11,$12,CASE WHEN $13 THEN 0 ELSE 1 END,$14::text::timestamptz,$14::text::timestamptz,$14::text::timestamptz FROM user_strategy_configs s WHERE s.id=$15")
        .bind(run).bind(db.owner_id).bind(job).bind(session).bind(hash('c')).bind(hash('d')).bind(hash('e')).bind(&admission).bind(hash('b')).bind(cohort).bind(hash('7')).bind(&target).bind(weight).bind(&now).bind(config).execute(&mut *tx).await?;
    sqlx::query("UPDATE jobs j SET payload_json=daily_0060_payload(r) FROM owner_beta_recommendation_runs r WHERE j.id=$1 AND r.id=$2").bind(job).bind(run).execute(&mut *tx).await?;
    // Mirror Daily publication: excluded rows retain numeric zero, no rank,
    // and a known exclusion reason. Historical NULL weights are not Daily.
    sqlx::query("INSERT INTO owner_beta_recommendation_items(recommendation_run_id,owner_user_id,instrument_id,rank,target_weight,excluded,exclusion_reason,reason_codes) SELECT $1,$2,id,CASE WHEN id='069500.KRX' AND $3 THEN 1 ELSE NULL END,CASE WHEN id='069500.KRX' AND $3 THEN 1 ELSE 0 END,NOT(id='069500.KRX' AND $3),CASE WHEN id='069500.KRX' AND $3 THEN NULL WHEN id='069500.KRX' THEN 'TREND_NEGATIVE_CASH' ELSE 'NOT_SELECTED_BY_STRATEGY' END,jsonb_build_array(CASE WHEN id='069500.KRX' AND $3 THEN 'TREND_POSITIVE' WHEN id='069500.KRX' THEN 'TREND_NEGATIVE_CASH' ELSE 'NOT_SELECTED_BY_STRATEGY' END) FROM instruments WHERE id=ANY(ARRAY['069500.KRX','102110.KRX','114260.KRX','132030.KRX','133690.KRX','143850.KRX','148070.KRX','153130.KRX','192090.KRX','195930.KRX','229200.KRX'])").bind(run).bind(db.owner_id).bind(weight).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_daily_reservations(owner_user_id,strategy_config_id,session_date,trigger_version,run_id,job_id,admission_sha256,strategy_config_sha256,config_epoch,activation_epoch) VALUES($1,$2,$3::text::date,'owner-beta-daily-trigger-v1',$4,$5,$6,$7,($8::jsonb->>'config_epoch')::bigint,($8::jsonb->>'activation_epoch')::bigint)").bind(db.owner_id).bind(config).bind(session).bind(run).bind(job).bind(&admission).bind(hash('c')).bind(cohort).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_daily_publication_claims(run_id,job_id,owner_user_id,attempt_id,attempt_no,claimed_by,locked_at,lease_seconds,authorized_at) VALUES($1,$2,$3,$4,1,'synthetic-source',$5::text::timestamptz,60,$5::text::timestamptz)").bind(run).bind(job).bind(db.owner_id).bind(attempt).bind(&now).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_paper_source_slots(slot_id,reservation_id,root_event_sha256,owner_user_id,policy_sha256,activation_epoch,session_date,cohort_sha256,cohort) VALUES($1,$2,$3,$4,$5,($6::jsonb->>'activation_epoch')::bigint,$7::text::date,$8,$6)").bind(&slot).bind(Uuid::new_v4()).bind(&root).bind(db.owner_id).bind(hash('b')).bind(cohort).bind(session).bind(cohort["cohort_sha256"].as_str().ok_or("fixture cohort digest")?).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_paper_source_admissions(slot_id,source_event_sha256,original_set_sha256,admission_sha256) VALUES($1,$2,$3,$4)").bind(&slot).bind(&source).bind(original).bind(&admission).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO owner_beta_paper_source_runs(slot_id,run_id,job_id,attempt_id) VALUES($1,$2,$3,$4)").bind(&slot).bind(run).bind(job).bind(attempt).execute(&mut *tx).await?;
    for table in tables {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE public.{table} ENABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await?;
    }
    let after: Value = sqlx::query_scalar(trigger_sql)
        .bind(&tables[..])
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(
        before, after,
        "synthetic setup must restore predecessor triggers"
    );
    tx.commit().await?;
    Ok(SqlWitness {
        session: session.into(),
        run,
        admission,
        root,
        source,
    })
}
const PAPER_0067_DOWN: &str =
    include_str!("../../../../migrations/0067_owner_beta_paper_ledger_v1.down.sql");
const PAPER_0068_DOWN: &str =
    include_str!("../../../../migrations/0068_owner_beta_paper_operations_v1.down.sql");
const PAPER_AUDIT_PROJECTION: &str =
    include_str!("../../../../docs/contracts/owner-beta-paper-audit-projection-v1.json");

#[test]
fn paper_0067_0068_sql_lifecycle_source_contract_is_closed() {
    assert_eq!(
        MIGRATOR
            .migrations
            .iter()
            .filter(|migration| migration.version == 68)
            .count(),
        2,
        "0068 must have one reserved reversible migration pair"
    );
    assert_eq!(
        PAPER_0068_UP
            .matches("CREATE TABLE public.owner_beta_paper_")
            .count(),
        3,
        "0068 owns exactly the operation, observation, and recovery-grant tables"
    );
    for signature in [
        "begin_owner_beta_paper_operation_v1(expected_epoch bigint,intent jsonb,request_key text) RETURNS jsonb",
        "inspect_owner_beta_paper_operation_v1(generation uuid,after_target uuid,page_limit integer) RETURNS jsonb",
        "reconcile_owner_beta_paper_operation_v1(generation uuid,expected_epoch bigint,inventory_sha256 text,request_key text) RETURNS jsonb",
    ] {
        assert!(
            PAPER_0068_UP.contains(signature),
            "missing SQL ABI: {signature}"
        );
    }
    for statement in [
        "ALTER TABLE public.%I ENABLE ROW LEVEL SECURITY",
        "ALTER TABLE public.%I FORCE ROW LEVEL SECURITY",
        "ALTER TABLE public.%I OWNER TO migration_owner",
        "REVOKE ALL ON public.%I FROM PUBLIC,app,worker",
    ] {
        assert!(PAPER_0068_UP.contains(statement));
    }
    assert!(PAPER_0068_UP.contains("session_user IS DISTINCT FROM 'migration_owner'"));
    assert!(PAPER_0068_UP.contains("current_setting('role',true) IS DISTINCT FROM 'none'"));
    assert!(PAPER_0068_UP.contains("public.paper_0068_canonical_json(intent)"));
    assert!(PAPER_0068_UP.contains("paper_0068_nonnegative_i64(value_text text)"));
    assert!(PAPER_0068_UP.contains("'paper_0068_nonnegative_i64(text)'"));
    assert!(PAPER_0068_UP.contains("c.control_epoch>=9223372036854775806"));
    assert!(PAPER_0068_UP.contains("octet_length(name)::text||':'||name"));
    assert!(PAPER_0068_UP.contains("public.paper_0068_canonical_row_json(item->'row')"));
    assert!(!PAPER_0068_UP.contains("daily_0060_digest(intent::text)"));
    assert!(
        !PAPER_0068_UP.contains("GRANT EXECUTE ON FUNCTION public.paper_0068_recovery_authority")
    );
    assert!(PAPER_0068_UP.contains("recovery_authority"));
    assert!(PAPER_0068_UP.contains("SET revoked_at=clock_timestamp()"));
    assert!(PAPER_0068_UP.contains("RECOVERY_AUTHORITY_UNAVAILABLE"));
    // These hooks are SQL string literals used to replace the serving bodies.
    assert!(
        PAPER_0068_UP.contains("settlement_receipt_sha256=settlement->>''calculation_sha256''")
    );
    assert!(PAPER_0068_UP.contains("valuation_receipt_sha256=valuation->>''calculation_sha256''"));
    assert!(
        PAPER_0068_UP.contains("earlier.account_id=c.account_id AND earlier.effective_session<s.effective_session AND (earlier.state<>'VALUED' OR earlier.valuation_sha256 IS NULL)")
    );
    assert!(PAPER_0068_DOWN.contains("owner_beta_paper_events"));
    assert!(PAPER_0068_DOWN.contains("DROP FUNCTION public.transition_owner_beta_paper_v1"));
    assert!(PAPER_AUDIT_PROJECTION.contains("\"reconciliation_v1\""));
    assert!(PAPER_AUDIT_PROJECTION.contains("\"archive_audit_v1\""));
    let projection: Value =
        serde_json::from_str(PAPER_AUDIT_PROJECTION).expect("valid projection JSON");
    assert_eq!(
        projection["reconciliation_v1"]["database_root_order"],
        json!([
            "account_strategy_bindings",
            "accounts",
            "jobs",
            "ledger",
            "outbox",
            "owner_beta_paper_control",
            "owner_beta_paper_events",
            "pending"
        ])
    );
    let groups = projection["archive_audit_v1"]["paper_source_table_projection"]
        .as_object()
        .expect("closed Paper audit groups");
    let catalogs = projection["archive_audit_v1"]["catalog_projection"]
        .as_object()
        .expect("explicit catalog projections");
    let catalog_names = groups["schema_security"]
        .as_array()
        .expect("security catalog inventory")
        .iter()
        .map(|table| table.as_str().expect("catalog name"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        catalog_names,
        catalogs
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        "every catalog projection belongs to the security group"
    );
    for (group, tables) in groups {
        for table in tables.as_array().expect("table list") {
            let table = table.as_str().expect("table name");
            if group == "schema_security" {
                let catalog = &catalogs[table];
                let columns = catalog["columns"].as_array().expect("catalog columns");
                assert!(!columns.is_empty(), "catalog columns for {table}");
                let names = columns
                    .iter()
                    .map(|column| column.as_str().expect("catalog column name"))
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(names.len(), columns.len(), "unique columns for {table}");
                let expressions = catalog["projection_sql"]
                    .as_object()
                    .expect("explicit catalog expressions");
                assert_eq!(
                    names,
                    expressions
                        .keys()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                );
                assert!(
                    ["source_sql", "key_sql"]
                        .iter()
                        .all(|field| catalog[*field].as_str().is_some_and(|sql| !sql.is_empty()))
                        && expressions
                            .values()
                            .all(|sql| sql.as_str().is_some_and(|sql| !sql.is_empty())),
                    "catalog source, row key and every selected expression for {table}"
                );
                continue;
            }
            assert!(
                projection["archive_audit_v1"]["archive_table_columns"][table]
                    .as_array()
                    .is_some_and(|columns| !columns.is_empty()),
                "Paper archive projection must freeze actual columns for {table}"
            );
            assert!(
                projection["archive_audit_v1"]["archive_identity_keys"][table].is_string(),
                "Paper archive projection must freeze primary-key order for {table}"
            );
            assert!(
                projection["archive_audit_v1"]["archive_identity_key_components"][table]
                    .as_array()
                    .is_some_and(|key| !key.is_empty()),
                "Paper archive projection must expose key components for {table}"
            );
        }
    }
}

async fn prepared_paper(db: &Daily0060Db, weight: bool) -> Daily0060Result<(SqlPaper, Value)> {
    let (account, config, _) = configure(db).await?;
    sqlx::query("UPDATE data_entitlements SET covered_uses='[\"recommendation\",\"paper_view\"]' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
    let (activation,epoch):(i64,i64)=sqlx::query_as("SELECT d.activation_epoch,s.daily_epoch FROM owner_beta_daily_control d JOIN user_strategy_configs s ON s.id=$1").bind(config).fetch_one(&db.owner).await?;
    let cohort = json!({"owner_user_id":db.owner_id.to_string(),"policy_sha256":hash('b'),"activation_epoch":activation,"strategy_config_id":config.to_string(),"config_epoch":epoch,"strategy_config_sha256":hash('c'),"entitlement_reference":"daily-fixture","entitlement_sha256":hash('a'),"cohort_sha256":hash('d'),"release_manifest_sha256":hash('e'),"research_image_id":hash('1'),"helper_image_id":hash('2'),"semantic_config_sha256":hash('3'),"calendar_sha256":hash('4'),"verifier_revision":"a".repeat(40)});
    let dates:Vec<String>=sqlx::query_scalar("SELECT ((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+n)::text FROM generate_series(-2,2) n ORDER BY n").fetch_all(&db.owner).await?;
    let days = Value::Array(
        dates
            .iter()
            .enumerate()
            .map(|(i, d)| json!({"date":d,"kind":"TRADING","ordinal":i,"row_hash":hash('4')}))
            .collect(),
    );
    calendar_claim(
        &mut *db.owner.acquire().await?,
        db.owner_id,
        &hash('5'),
        &hash('b'),
        &days,
        false,
    )
    .await?;
    let mut witnesses = Vec::new();
    let mut parent: Option<String> = None;
    for day in dates.iter().take(3) {
        let w = seed_witness(db, config, &cohort, day, parent.as_deref(), weight).await?;
        parent = Some(w.admission.clone());
        witnesses.push(w);
    }
    let consent:Value=sqlx::query_scalar("SELECT record_owner_beta_paper_consent_v1($1,0,'owner-beta-paper-v1-draft-1',$2,true,'enable-consent')").bind(account).bind(financial_contract()["contract_sha256"].as_str().unwrap()).fetch_one(&db.app).await?;
    let digest = hash('6');
    let binding = json!({"schema":"owner-beta-paper-process-v1","owner_user_id":db.owner_id.to_string(),"account_id":account.to_string(),"strategy_config_id":config.to_string(),"consent_event_id":consent["data"]["event_id"],"consent_version":"owner-beta-paper-v1-draft-1","control_epoch":1,"daily_cohort_sha256":hash('d'),"financial_contract_sha256":financial_contract()["contract_sha256"],"code_commit":"b".repeat(40),"release_manifest_sha256":hash('c'),"api_binary_sha256":hash('d'),"worker_binary_sha256":hash('e')});
    let (now, expires): (String, String) = sqlx::query_as(
        "SELECT to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),to_char((clock_timestamp()+interval '120 seconds') AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .fetch_one(&db.owner)
    .await?;
    let evidence = json!({"current_cohort":cohort,"process_binding":binding,"process_binding_sha256":digest,"evaluated_at":now,"expires_at":expires,"expected_epoch":0,"new_epoch":1,"session_evidence":witnesses.iter().map(|w|json!({"session_date":w.session,"run_id":w.run.to_string(),"admission_sha256":w.admission,"source_event_sha256":w.source,"root_event_sha256":w.root})).collect::<Vec<_>>()});
    Ok((
        SqlPaper {
            account,
            config,
            cohort,
            digest,
            witness: witnesses.pop().ok_or("witness")?,
        },
        evidence,
    ))
}

async fn enabled_paper(db: &Daily0060Db, weight: bool) -> Daily0060Result<SqlPaper> {
    let (paper, evidence) = prepared_paper(db, weight).await?;
    let mut invalid = evidence.clone();
    invalid["session_evidence"][2] = invalid["session_evidence"][1].clone();
    denied(
        sqlx::query("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'duplicate-evidence')")
            .bind(invalid)
            .execute(&db.owner)
            .await,
        "23514",
    );
    let enabled = transition_with_actor_checks(db, 0, "ENABLE", &evidence, "enable").await?;
    assert_eq!(enabled["outcome"], "OK");
    Ok(paper)
}

// Independent encoder for the ASCII-only exact-request fixtures. Sorting here
// must agree with PostgreSQL, including escaped embedded timestamp text.
fn exact_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        exact_json(&map[k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(exact_json).collect::<Vec<_>>().join(",")
        ),
        _ => serde_json::to_string(v).unwrap(),
    }
}

fn exact_request(kind: &str, account: Option<Uuid>, args: Value) -> Value {
    json!({"schema":"owner-beta-paper-exact-request-v1","kind":kind,"account_id":account,"args":args})
}

fn transition_request(account: Uuid, epoch: i64, kind: &str, evidence: &Value, key: &str) -> Value {
    exact_request(
        "TRANSITION",
        Some(account),
        json!({"expected_epoch":epoch,"transition":kind,"evidence_canonical":exact_json(evidence),"request_key":key}),
    )
}

async fn exact_query(
    connection: &mut sqlx::PgConnection,
    request: &Value,
) -> Result<Value, sqlx::Error> {
    // Timeouts are set by the client before SELECT as well as by the function.
    sqlx::raw_sql("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='15s'")
        .execute(&mut *connection)
        .await?;
    let wire = exact_json(request);
    sqlx::query_scalar("SELECT read_owner_beta_paper_operation_result_v1($1,daily_0060_digest($1))")
        .bind(wire)
        .fetch_one(connection)
        .await
}

async fn exact_read(pool: &PgPool, request: &Value) -> Daily0060Result<Value> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let result = exact_query(&mut tx, request).await?;
    tx.rollback().await?;
    Ok(result)
}

async fn replay_exact_call(
    connection: &mut sqlx::PgConnection,
    request: &Value,
) -> Result<Value, sqlx::Error> {
    let a = &request["args"];
    let key = a["request_key"].as_str().unwrap();
    match request["kind"].as_str().unwrap() {
        "CONFIGURE" => {
            sqlx::query_scalar("SELECT configure_owner_beta_paper_v1($1,$2,$3,NULL,$4)")
                .bind(Uuid::parse_str(a["strategy_config_id"].as_str().unwrap()).unwrap())
                .bind(a["account_name"].as_str().unwrap())
                .bind(
                    serde_json::from_str::<Value>(a["contract_canonical"].as_str().unwrap())
                        .unwrap(),
                )
                .bind(key)
                .fetch_one(connection)
                .await
        }
        "BEGIN" => {
            sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)")
                .bind(a["expected_epoch"].as_i64().unwrap())
                .bind(
                    serde_json::from_str::<Value>(a["intent_canonical"].as_str().unwrap()).unwrap(),
                )
                .bind(key)
                .fetch_one(connection)
                .await
        }
        "RECONCILE" => {
            sqlx::query_scalar("SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)")
                .bind(Uuid::parse_str(a["generation"].as_str().unwrap()).unwrap())
                .bind(a["expected_epoch"].as_i64().unwrap())
                .bind(a["inventory_sha256"].as_str().unwrap())
                .bind(key)
                .fetch_one(connection)
                .await
        }
        _ => {
            sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
                .bind(a["expected_epoch"].as_i64().unwrap())
                .bind(a["transition"].as_str().unwrap())
                .bind(
                    serde_json::from_str::<Value>(a["evidence_canonical"].as_str().unwrap())
                        .unwrap(),
                )
                .bind(key)
                .fetch_one(connection)
                .await
        }
    }
}

async fn operation_image(db: &Daily0060Db) -> Daily0060Result<Value> {
    let mut image = serde_json::Map::new();
    // Complete rows, including receipt text, ledger, jobs and both outbox stores.
    // Only compile-time table names enter SQL; there is no caller selector.
    for table in [
        "owner_beta_paper_control",
        "owner_beta_paper_events",
        "owner_beta_paper_operations",
        "owner_beta_paper_operation_observations",
        "owner_beta_paper_recovery_grants",
        "accounts",
        "account_strategy_bindings",
        "cash_ledger",
        "positions",
        "orders",
        "fills",
        "daily_equity",
        "pending_targets",
        "paper_rebalance_previews",
        "owner_beta_paper_lineage",
        "owner_beta_paper_sessions",
        "owner_beta_paper_session_events",
        "paper_settlement_outbox",
        "paper_settlement_outbox_archive",
        "notifications",
        "notification_deliveries",
        "jobs",
        "job_attempts",
        "owner_beta_paper_source_slots",
        "owner_beta_paper_source_admissions",
        "owner_beta_paper_source_runs",
        "owner_beta_daily_reservations",
    ] {
        let rows: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text COLLATE \"C\"),'[]'::jsonb) FROM public.{table} t"))).fetch_one(&db.owner).await?;
        image.insert(table.into(), rows);
    }
    Ok(Value::Object(image))
}

async fn assert_exact_receipt(
    db: &Daily0060Db,
    request: &Value,
    original: &Value,
) -> Daily0060Result {
    let before = operation_image(db).await?;
    let caller = login_without_actor(db, "migration_owner", 1).await?;
    let result: Daily0060Result = async {
        let foreign = Uuid::new_v4().to_string();
        for actor in [None, Some(""), Some(foreign.as_str())] {
            let mut tx = caller.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await?;
            if let Some(actor) = actor {
                sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
                    .bind(actor)
                    .execute(&mut *tx)
                    .await?;
            }
            let prior: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *tx)
                    .await?;
            let read = exact_query(&mut tx, request).await?;
            assert_eq!(read["status"], "COMMITTED");
            assert_eq!(read["result_canonical"], exact_json(original));
            let digest: String = sqlx::query_scalar("SELECT daily_0060_digest($1)")
                .bind(exact_json(original))
                .fetch_one(&mut *tx)
                .await?;
            assert_eq!(read["result_sha256"], digest);
            assert_eq!(read["account_id"], original["account_id"]);
            assert_eq!(read["event_epoch"], original["control_epoch"]);
            let id = match request["kind"].as_str().unwrap() {
                "BEGIN" => &original["data"]["operation_event_id"],
                "RECONCILE" => &original["data"]["observation_id"],
                _ => &original["data"]["event_id"],
            };
            assert_eq!(&read["event_id"], id);
            assert_actor(&mut tx, &prior).await?;
            assert_eq!(read, exact_query(&mut tx, request).await?);
            assert_actor(&mut tx, &prior).await?;
            assert_eq!(
                replay_exact_call(&mut tx, request).await?["outcome"],
                "REPLAY"
            );
            assert_actor(&mut tx, &prior).await?;

            let mut changed = request.clone();
            if request["kind"] == "CONFIGURE" {
                changed["args"]["account_name"] = json!("Changed exact name");
            } else {
                changed["args"]["expected_epoch"] =
                    json!(request["args"]["expected_epoch"].as_i64().unwrap() + 1);
            }
            sqlx::query("SAVEPOINT actor_error")
                .execute(&mut *tx)
                .await?;
            let error = exact_query(&mut tx, &changed).await;
            actor_error(&mut tx, error, "23505", &prior).await?;
            sqlx::query("SAVEPOINT actor_error")
                .execute(&mut *tx)
                .await?;
            let error = replay_exact_call(&mut tx, &changed).await;
            actor_error(&mut tx, error, "23505", &prior).await?;
            tx.rollback().await?;
        }
        Ok(())
    }
    .await;
    caller.close().await;
    result?;
    assert_eq!(
        before,
        operation_image(db).await?,
        "exact receipt reads/conflicts must not write any row"
    );
    Ok(())
}

async fn assert_exact_transition(
    db: &Daily0060Db,
    account: Uuid,
    epoch: i64,
    kind: &str,
    evidence: &Value,
    key: &str,
    original: &Value,
) -> Daily0060Result {
    assert_exact_receipt(
        db,
        &transition_request(account, epoch, kind, evidence, key),
        original,
    )
    .await?;
    let before = operation_image(db).await?;
    let mut tx = db.owner.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let replay: Value = sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
        .bind(epoch)
        .bind(kind)
        .bind(evidence)
        .bind(key)
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(replay["outcome"], "REPLAY");
    sqlx::query("SAVEPOINT changed_epoch")
        .execute(&mut *tx)
        .await?;
    denied(
        sqlx::query_scalar::<_, Value>("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
            .bind(epoch + 1)
            .bind(kind)
            .bind(evidence)
            .bind(key)
            .fetch_one(&mut *tx)
            .await,
        "23505",
    );
    tx.rollback().await?;
    assert_eq!(before, operation_image(db).await?);
    Ok(())
}

async fn assert_legacy_unproven(db: &Daily0060Db, request: &Value) -> Daily0060Result {
    let (table, trigger) = match request["kind"].as_str().unwrap() {
        "BEGIN" => (
            "owner_beta_paper_operations",
            "owner_beta_paper_operations_immutable",
        ),
        "RECONCILE" => (
            "owner_beta_paper_operation_observations",
            "owner_beta_paper_observations_immutable",
        ),
        _ => (
            "owner_beta_paper_events",
            "owner_beta_paper_events_immutable",
        ),
    };
    let mut tx = db.owner.begin().await?;
    // Model a genuine pre-amendment row without inventing a preimage. These
    // trusted fixture changes are rolled back, never used as migration evidence.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE public.{table} DISABLE TRIGGER {trigger}"
    )))
    .execute(&mut *tx)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE public.{table} SET request_canonical=NULL,request_sha256=NULL,result_canonical=NULL,result_sha256=NULL WHERE request_key=$1"))).bind(request["args"]["request_key"].as_str().unwrap()).execute(&mut *tx).await?;
    let row_sql =
        format!("SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM public.{table} t");
    let before: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(row_sql.clone()))
        .fetch_one(&mut *tx)
        .await?;
    let read = exact_query(&mut tx, request).await?;
    assert_eq!(read["status"], "UNPROVEN");
    for field in [
        "event_id",
        "event_epoch",
        "result_canonical",
        "result_sha256",
        "reset",
    ] {
        assert_eq!(read[field], Value::Null);
    }
    let after: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(row_sql))
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(before, after, "legacy read must not backfill a receipt");
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_exact_receipt_canonical_roles_legacy_and_down() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 67).await?;
    let result: Daily0060Result = async {
        let (account,config,_) = configure(&db).await?;
        let legacy: Value = sqlx::query_scalar("SELECT to_jsonb(e) FROM owner_beta_paper_events e WHERE kind='CONFIGURED'").fetch_one(&db.owner).await?;
        let catalog_before=paper_catalog(&mut *db.owner.acquire().await?).await?;
        let mut install = db.owner.begin().await?;
        sqlx::raw_sql(PAPER_0068_UP).execute(&mut *install).await?;
        install.commit().await?;
        let attributes:bool=sqlx::query_scalar("SELECT p.prosecdef AND r.rolname='migration_owner' AND p.proconfig @> ARRAY['lock_timeout=5s','statement_timeout=15s'] AND EXISTS(SELECT 1 FROM unnest(p.proconfig) c WHERE replace(c,' ','')='search_path=pg_catalog,pg_temp') FROM pg_proc p JOIN pg_roles r ON r.oid=p.proowner WHERE p.oid='public.read_owner_beta_paper_operation_result_v1(text,text)'::regprocedure").fetch_one(&db.owner).await?;
        assert!(attributes,"exact reader must retain its definer, owner, safe path and timeout contract");
        let request=exact_request("CONFIGURE",None,json!({"strategy_config_id":config,"account_name":"SQL fixture","contract_canonical":exact_json(&financial_contract()),"process_binding":null,"request_key":"sql-config"}));
        let before=operation_image(&db).await?;
        let read=exact_read(&db.owner,&request).await?;
        assert_eq!(read["status"],"UNPROVEN");
        assert_eq!(read["event_id"],Value::Null);
        assert_eq!(read["result_canonical"],Value::Null);
        assert_eq!(before,operation_image(&db).await?);
        let preserved: Value = sqlx::query_scalar("SELECT to_jsonb(e)-ARRAY['request_canonical','request_sha256','result_canonical','result_sha256'] FROM owner_beta_paper_events e WHERE kind='CONFIGURED'").fetch_one(&db.owner).await?;
        assert_eq!(legacy,preserved,"migration must not backfill guessed legacy evidence");
        let mut conflict=request.clone(); conflict["args"]["account_name"]=json!("Changed legacy name");
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&conflict).await,"23505"); tx.rollback().await?;
        let reset=exact_read(&db.owner,&exact_request("RESET",Some(account),json!({"expected_epoch":0}))).await?;
        assert_eq!(reset["reset"]["event_id"],legacy["id"]);
        assert_eq!(reset["reset"]["kind"],"CONFIGURED");
        let caller=login_without_actor(&db,"migration_owner",1).await?;
        let foreign=Uuid::new_v4().to_string();
        for actor in [None,Some(""),Some(foreign.as_str())] {
            let mut tx=caller.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *tx).await?;
            if let Some(actor)=actor {sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(actor).execute(&mut *tx).await?;}
            let prior:Option<String>=sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *tx).await?;
            assert_eq!(exact_query(&mut tx,&request).await?["status"],"UNPROVEN"); assert_actor(&mut tx,&prior).await?;
            let mut absent=request.clone(); absent["args"]["request_key"]=json!("never-configured");
            assert_eq!(exact_query(&mut tx,&absent).await?["status"],"ABSENT"); assert_actor(&mut tx,&prior).await?;
            assert_eq!(exact_query(&mut tx,&exact_request("RESET",Some(account),json!({"expected_epoch":0}))).await?["status"],"RESET"); assert_actor(&mut tx,&prior).await?;
            sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
            let error=exact_query(&mut tx,&exact_request("RESET",Some(account),json!({"expected_epoch":1}))).await;
            actor_error(&mut tx,error,"40001",&prior).await?;
            tx.rollback().await?;
        }
        caller.close().await;

        let (wire,row,root):(String,String,String)=sqlx::query_as("SELECT paper_0068_canonical_json('{\"z\":[true,false,null,{\"a\":true}],\"a\":false}'::jsonb),paper_0068_canonical_row_json('{\"a\":[true,false,null],\"n\":2.00,\"s\":\"x\"}'::jsonb),paper_0068_stream_digest('[{\"table\":\"probe\",\"row_key\":\"1\",\"row\":{\"a\":[true,false,null],\"n\":2.00,\"s\":\"x\"}}]'::jsonb)").fetch_one(&db.owner).await?;
        assert_eq!(wire,"{\"a\":false,\"z\":[true,false,null,{\"a\":true}]}");
        assert_eq!(row,"{\"a\":[true,false,null],\"n\":2,\"s\":\"x\"}");
        assert_eq!(root,"sha256:ec15672aeec8f264267dc07b2c378915763797f89b4ae99eb68c6169ba6849bb");

        for principal in ["app","worker","admin","audit_writer","research_writer","daily_input_admitter"] {
            let pool=login_without_actor(&db,principal,1).await?;
            let mut tx=pool.begin().await?;
            denied(exact_query(&mut tx,&request).await,"42501");
            tx.rollback().await?;
            let granted: bool=sqlx::query_scalar("SELECT has_function_privilege($1,'public.read_owner_beta_paper_operation_result_v1(text,text)','EXECUTE')").bind(principal).fetch_one(&db.owner).await?;
            assert!(!granted);
            pool.close().await;
        }
        let supervisor=admin_pool(&super::supervisor_db_url(&db.url,&db.name)).await.map_err(|e|std::io::Error::other(e.to_string()))?;
        let mut tx=supervisor.begin().await?;
        denied(exact_query(&mut tx,&request).await,"42501"); tx.rollback().await?;
        supervisor.close().await;
        let mut tx=db.owner.begin().await?;
        sqlx::query("SET LOCAL ROLE migration_owner").execute(&mut *tx).await?;
        denied(exact_query(&mut tx,&request).await,"42501"); tx.rollback().await?;
        denied(sqlx::query("SELECT request_canonical FROM owner_beta_paper_events").execute(&db.app).await,"42501");
        let original_columns:i64=sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_events WHERE account_id=$1").bind(account).fetch_one(&db.app).await?;
        assert_eq!(original_columns,1);

        let canonical=exact_json(&request);
        let mut invalid=vec![format!(" {canonical}"),format!("{canonical}\n"),canonical.replacen("{","{\"schema\":\"duplicate\",",1),"x".repeat(1048577)];
        let mut bad=request.clone(); bad["args"]["contract_canonical"]=json!(format!(" {}",exact_json(&financial_contract()))); invalid.push(exact_json(&bad));
        let mut bad=request.clone(); bad["args"]["contract_canonical"]=json!("{\"x\":true,\"x\":false}"); invalid.push(exact_json(&bad));
        let mut bad=request.clone(); bad["args"]["contract_canonical"]=json!(format!("{}0{}","[".repeat(17),"]".repeat(17))); invalid.push(exact_json(&bad));
        let mut bad=request.clone(); bad["args"]["extra"]=json!(null); invalid.push(exact_json(&bad));
        for wire in invalid {
            let mut tx=db.owner.begin().await?;
            denied(sqlx::query_scalar::<_,Value>("SELECT read_owner_beta_paper_operation_result_v1($1,daily_0060_digest($1))").bind(wire).fetch_one(&mut *tx).await,"22023"); tx.rollback().await?;
        }
        let mut tx=db.owner.begin().await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT read_owner_beta_paper_operation_result_v1($1,$2)").bind(canonical).bind(hash('0')).fetch_one(&mut *tx).await,"22023"); tx.rollback().await?;
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&exact_request("RESET",Some(Uuid::new_v4()),json!({"expected_epoch":0}))).await,"42501"); tx.rollback().await?;
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&exact_request("RESET",Some(account),json!({"expected_epoch":1}))).await,"40001"); tx.rollback().await?;

        // Pristine 0068 down preserves pre-existing 0067 history and its grants.
        let mut down=db.owner.begin().await?;
        sqlx::raw_sql(PAPER_0068_DOWN).execute(&mut *down).await?;
        let after:Value=sqlx::query_scalar("SELECT to_jsonb(e) FROM owner_beta_paper_events e WHERE kind='CONFIGURED'").fetch_one(&mut *down).await?;
        assert_eq!(after,legacy);
        let catalog_after=paper_catalog(&mut down).await?;
        for section in ["tables","columns","constraints","indexes","policies","triggers","functions"] {
            assert_eq!(catalog_before[section],catalog_after[section],"0068 down must restore original 0067 {section}");
        }
        down.rollback().await?;
        Ok(())
    }.await;
    db.close().await?;
    result
}

fn sql_operation_intent(
    account: Uuid,
    generation: Uuid,
    predecessor: Option<Uuid>,
    epoch: i64,
) -> Value {
    json!({"schema":"owner-beta-paper-operation-intent-v1","installation_id":Uuid::new_v4(),"generation":generation,"account_id":account,"predecessor_generation":predecessor,"request_key":format!("{generation}:begin"),"purpose":"STOP","scope":"PAPER_STOP","expected_epoch":epoch,"local_epoch_floor":epoch,"release_manifest_sha256":hash('a'),"artifact_sha256":null,"resume_target_id":null})
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_exact_readback_live_backend_late_commit_and_cancel() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let caller = login_without_actor(&db, "migration_owner", 3).await?;
    let result: Daily0060Result=async {
        let unconfigured=exact_request("CONFIGURE",None,json!({"strategy_config_id":Uuid::new_v4(),"account_name":"Not submitted","contract_canonical":exact_json(&financial_contract()),"process_binding":null,"request_key":"never-before-configure"}));
        let absent=exact_read(&caller,&unconfigured).await?;
        assert_eq!(absent["status"],"ABSENT");
        for field in ["account_id","current_epoch","current_state","event_id","event_epoch","result_canonical","result_sha256","reset"] {assert_eq!(absent[field],Value::Null);}
        let (account,_,_)=configure(&db).await?;
        let generation=Uuid::new_v4();
        let intent=sql_operation_intent(account,generation,None,0);
        let request=exact_request("BEGIN",Some(account),json!({"expected_epoch":0,"intent_canonical":exact_json(&intent),"request_key":intent["request_key"]}));
        let mut writer=caller.acquire().await?;
        let writer_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *writer).await?;
        sqlx::query("BEGIN").execute(&mut *writer).await?;
        let original:Value=sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1(0,$1,$2)").bind(&intent).bind(intent["request_key"].as_str().unwrap()).fetch_one(&mut *writer).await?;
        // Different, captured reader backend, with a snapshot that stays old.
        let mut reader=caller.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *reader).await?;
        let reader_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *reader).await?;
        assert_ne!(writer_pid,reader_pid);
        assert_eq!(exact_query(&mut reader,&request).await?["status"],"ABSENT");
        let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND xact_start IS NOT NULL)").bind(writer_pid).fetch_one(&db.owner).await?;
        assert!(live,"ABSENT while the original transaction is live is uncertainty");
        // A caller deadline does not terminate the owned writer task. It commits
        // after that deadline; we never resubmit its mutation or choose a new key.
        let mut commit=tokio::spawn(async move {
            sqlx::raw_sql("SELECT pg_sleep(0.3); COMMIT").execute(&mut *writer).await
        });
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20),&mut commit).await.is_err());
        assert_eq!(exact_query(&mut reader,&request).await?["status"],"ABSENT");
        commit.await??;
        assert_eq!(exact_query(&mut reader,&request).await?["status"],"ABSENT","an old snapshot cannot prove final absence");
        reader.rollback().await?;
        assert_exact_receipt(&db,&request,&original).await?;
        let mut collision=intent.clone(); collision["generation"]=json!(Uuid::new_v4());
        let mut tx=db.owner.begin().await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT begin_owner_beta_paper_operation_v1(0,$1,$2)").bind(&collision).bind(intent["request_key"].as_str().unwrap()).fetch_one(&mut *tx).await,"23505"); tx.rollback().await?;
        let collision_request=exact_request("BEGIN",Some(account),json!({"expected_epoch":0,"intent_canonical":exact_json(&collision),"request_key":intent["request_key"]}));
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&collision_request).await,"23505"); tx.rollback().await?;
        // Commit-before-observation (including an ignored/lost client result)
        // returns the identical original OK bytes in fresh read-only snapshots.
        assert_eq!(exact_read(&caller,&request).await?["result_canonical"],exact_json(&original));

        let cancelled=Uuid::new_v4();
        let intent=sql_operation_intent(account,cancelled,Some(generation),1);
        let request=exact_request("BEGIN",Some(account),json!({"expected_epoch":1,"intent_canonical":exact_json(&intent),"request_key":intent["request_key"]}));
        let mut writer=caller.acquire().await?;
        let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *writer).await?;
        sqlx::query("BEGIN").execute(&mut *writer).await?;
        sqlx::query("SELECT begin_owner_beta_paper_operation_v1(1,$1,$2)").bind(&intent).bind(intent["request_key"].as_str().unwrap()).execute(&mut *writer).await?;
        assert_eq!(exact_read(&caller,&request).await?["status"],"ABSENT");
        let wait=tokio::spawn(async move {
            let result=sqlx::query("SELECT pg_sleep(15)").execute(&mut *writer).await;
            (writer,result)
        });
        let mut sleeping=false;
        for _ in 0..80 {
            sleeping=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event='PgSleep')").bind(pid).fetch_one(&db.owner).await?;
            if sleeping {break;}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        // Always cancel/reap our captured writer, even if observation failed.
        let sent:bool=sqlx::query_scalar("SELECT pg_cancel_backend($1)").bind(pid).fetch_one(&caller).await?;
        let (mut writer,interrupted)=wait.await?;
        assert!(sleeping && sent);
        denied(interrupted,"57014");
        sqlx::query("ROLLBACK").execute(&mut *writer).await?;
        assert_eq!(exact_read(&caller,&request).await?["status"],"ABSENT");
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_operations WHERE generation=$1").bind(cancelled).fetch_one(&db.owner).await?;
        assert_eq!(count,0,"only the same-connection rollback ACK proves this cancellation rolled back");
        Ok(())
    }.await;
    caller.close().await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_enable_reset_times_swaps_and_descriptive_readback() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result: Daily0060Result=async {
        let (paper,evidence)=prepared_paper(&db,false).await?;
        let request=exact_request("RESET",Some(paper.account),json!({"expected_epoch":0}));
        let reset=exact_read(&db.owner,&request).await?;
        assert_eq!(reset["status"],"RESET");
        assert_eq!(reset["reset"]["kind"],"CONFIGURED");
        let reset_us=reset["reset"]["occurred_at_us"].as_i64().unwrap();
        let exact_us:i64=sqlx::query_scalar("SELECT (extract(epoch FROM occurred_at)*1000000)::bigint FROM owner_beta_paper_events WHERE id=$1").bind(Uuid::parse_str(reset["reset"]["event_id"].as_str().unwrap())?).fetch_one(&db.owner).await?;
        assert_eq!(reset_us,exact_us);
        let before=operation_image(&db).await?;
        let collision=json!({"schema":"owner-beta-paper-drain-v1","expected_epoch":0,"reason_code":"OWNER_STOP"});
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&transition_request(paper.account,0,"DRAIN",&collision,"sql-config")).await,"23505"); tx.rollback().await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'DRAIN',$1,'sql-config')").bind(collision).fetch_one(&db.owner).await,"23505");
        // Trusted synthetic corruption is confined to rolled-back fixture
        // transactions; it is never claimed as genuine ARM/Root evidence.
        for (table,column,predicate) in [
            ("owner_beta_paper_source_slots","reserved_at","slot_id=(SELECT slot_id FROM owner_beta_paper_source_runs WHERE run_id=$1)"),
            ("owner_beta_daily_reservations","created_at","run_id=$1"),
            ("owner_beta_recommendation_runs","created_at","id=$1"),
            ("owner_beta_paper_source_admissions","linked_at","slot_id=(SELECT slot_id FROM owner_beta_paper_source_runs WHERE run_id=$1)"),
            ("owner_beta_paper_source_runs","linked_at","run_id=$1"),
        ] {
            for offset_us in [-1_i64,0,86_400_000_000] {
                let mut tx=db.owner.begin().await?;
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE public.{table} DISABLE TRIGGER USER"))).execute(&mut *tx).await?;
                let update=format!("UPDATE public.{table} SET {column}=timestamptz 'epoch'+$2::bigint*interval '1 microsecond' WHERE {predicate}");
                assert_eq!(sqlx::query(sqlx::AssertSqlSafe(update)).bind(paper.witness.run).bind(reset_us+offset_us).execute(&mut *tx).await?.rows_affected(),1);
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE public.{table} ENABLE TRIGGER USER"))).execute(&mut *tx).await?;
                denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'bad-time')").bind(&evidence).fetch_one(&mut *tx).await,"23514");
                tx.rollback().await?;
                assert_eq!(before,operation_image(&db).await?,"rejected {table}.{column} at offset {offset_us} changed history");
            }
        }
        for field in ["run_id","admission_sha256","source_event_sha256","root_event_sha256","session_date"] {
            let mut changed=evidence.clone();
            changed["session_evidence"][2][field]=changed["session_evidence"][0][field].clone();
            denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'swapped-projection')").bind(changed).fetch_one(&db.owner).await,"23514");
        }
        for swap in ["job","attempt"] {
            let mut tx=db.owner.begin().await?;
            sqlx::query("ALTER TABLE owner_beta_paper_source_runs DISABLE TRIGGER USER").execute(&mut *tx).await?;
            // Swap two valid relational identities while keeping all FK/unique
            // constraints valid, so the ENABLE join itself must reject them.
            let rows:Value=sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(s) ORDER BY s.slot_id) FROM owner_beta_paper_source_runs s").fetch_one(&mut *tx).await?;
            let mut rows=rows.as_array().unwrap().clone();
            let key=if swap=="job" {"job_id"} else {"attempt_id"};
            let tmp=rows[0][key].clone(); rows[0][key]=rows[1][key].clone(); rows[1][key]=tmp;
            sqlx::query("DELETE FROM owner_beta_paper_source_runs").execute(&mut *tx).await?;
            for row in rows {
                sqlx::query("INSERT INTO owner_beta_paper_source_runs SELECT * FROM jsonb_populate_record(NULL::owner_beta_paper_source_runs,$1)").bind(row).execute(&mut *tx).await?;
            }
            sqlx::query("ALTER TABLE owner_beta_paper_source_runs ENABLE TRIGGER USER").execute(&mut *tx).await?;
            denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'swapped-chain')").bind(&evidence).fetch_one(&mut *tx).await,"23514");
            tx.rollback().await?;
        }
        for corruption in [
            "UPDATE owner_beta_paper_source_slots SET cohort=jsonb_set(cohort,'{calendar_sha256}',to_jsonb(('sha256:'||repeat('9',64))::text))",
            "UPDATE owner_beta_daily_reservations SET strategy_config_sha256='sha256:'||repeat('9',64)",
        ] {
            let mut tx=db.owner.begin().await?;
            sqlx::raw_sql("ALTER TABLE owner_beta_paper_source_slots DISABLE TRIGGER USER; ALTER TABLE owner_beta_daily_reservations DISABLE TRIGGER USER").execute(&mut *tx).await?;
            sqlx::query(sqlx::AssertSqlSafe(corruption.to_string())).execute(&mut *tx).await?;
            sqlx::raw_sql("ALTER TABLE owner_beta_paper_source_slots ENABLE TRIGGER USER; ALTER TABLE owner_beta_daily_reservations ENABLE TRIGGER USER").execute(&mut *tx).await?;
            denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'swapped-cohort-or-reservation')").bind(&evidence).fetch_one(&mut *tx).await,"23514"); tx.rollback().await?;
        }
        let mut tx=db.owner.begin().await?;
        sqlx::query("ALTER TABLE owner_beta_paper_events DISABLE TRIGGER owner_beta_paper_events_immutable").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM owner_beta_paper_events WHERE kind='CONFIGURED'").execute(&mut *tx).await?;
        denied(exact_query(&mut tx,&request).await,"55000"); tx.rollback().await?;
        // No UUID/time ordering can resolve a duplicate maximal reset epoch.
        let mut tx=db.owner.begin().await?;
        sqlx::query("INSERT INTO owner_beta_paper_events(owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key) VALUES($1,$2,0,'OFF','{}',daily_0060_digest('{}'),'synthetic-tied-reset')").bind(db.owner_id).bind(paper.account).execute(&mut *tx).await?;
        sqlx::query("SAVEPOINT tied_reader").execute(&mut *tx).await?;
        denied(exact_query(&mut tx,&request).await,"55000");
        sqlx::query("ROLLBACK TO SAVEPOINT tied_reader").execute(&mut *tx).await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'tied-reset-enable')").bind(&evidence).fetch_one(&mut *tx).await,"55000"); tx.rollback().await?;
        let mut tx=db.owner.begin().await?;
        sqlx::query("INSERT INTO owner_beta_paper_events(owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key) VALUES($1,$2,1,'OFF','{}',daily_0060_digest('{}'),'synthetic-future-reset')").bind(db.owner_id).bind(paper.account).execute(&mut *tx).await?;
        denied(exact_query(&mut tx,&request).await,"55000"); tx.rollback().await?;
        let mut tx=db.owner.begin().await?;
        sqlx::query("UPDATE owner_beta_paper_control SET control_epoch=2").execute(&mut *tx).await?;
        let later:Uuid=sqlx::query_scalar("INSERT INTO owner_beta_paper_events(owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,occurred_at) SELECT $1,$2,2,'OFF','{}',daily_0060_digest('{}'),'synthetic-wall-rollback',occurred_at-interval '1 second' FROM owner_beta_paper_events WHERE kind='CONFIGURED' RETURNING id").bind(db.owner_id).bind(paper.account).fetch_one(&mut *tx).await?;
        let selected=exact_query(&mut tx,&exact_request("RESET",Some(paper.account),json!({"expected_epoch":2}))).await?;
        assert_eq!(selected["reset"]["event_id"],later.to_string()); tx.rollback().await?;
        assert_eq!(before,operation_image(&db).await?);
        let enabled:Value=sqlx::query_scalar("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'exact-enable')").bind(&evidence).fetch_one(&db.owner).await?;
        assert_exact_transition(&db,paper.account,0,"ENABLE",&evidence,"exact-enable",&enabled).await?;
        let timestamp=evidence["evaluated_at"].as_str().unwrap();
        assert_eq!(timestamp.split('.').nth(1).unwrap().len(),7,"fixture retains six fractional digits and Z");
        let mut reformatted=evidence.clone(); reformatted["evaluated_at"]=json!(timestamp.replace('Z',"+00:00"));
        let same_instant:bool=sqlx::query_scalar("SELECT $1::text::timestamptz=$2::text::timestamptz").bind(timestamp).bind(reformatted["evaluated_at"].as_str().unwrap()).fetch_one(&db.owner).await?;
        assert!(same_instant);
        let mut tx=db.owner.begin().await?;
        denied(exact_query(&mut tx,&transition_request(paper.account,0,"ENABLE",&reformatted,"exact-enable")).await,"23505"); tx.rollback().await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'exact-enable')").bind(reformatted).fetch_one(&db.owner).await,"23505");

        // Corruption and incomplete receipts fail closed; immutable rows are
        // changed only inside this trusted, rolled-back negative fixture.
        for corruption in [
            "UPDATE owner_beta_paper_events SET result_sha256='sha256:'||repeat('9',64) WHERE request_key='exact-enable'",
            "UPDATE owner_beta_paper_events SET request_sha256='sha256:'||repeat('9',64) WHERE request_key='exact-enable'",
            "UPDATE owner_beta_paper_events SET result_canonical=paper_0068_canonical_json(jsonb_set(result_canonical::jsonb,'{data,event_id}',to_jsonb(gen_random_uuid()::text))) WHERE request_key='exact-enable'; UPDATE owner_beta_paper_events SET result_sha256=daily_0060_digest(result_canonical) WHERE request_key='exact-enable'",
            "UPDATE owner_beta_paper_events SET request_canonical=paper_0068_canonical_json(jsonb_set(request_canonical::jsonb,'{args,expected_epoch}','1')) WHERE request_key='exact-enable'; UPDATE owner_beta_paper_events SET request_sha256=daily_0060_digest(request_canonical) WHERE request_key='exact-enable'",
        ] {
            let mut tx=db.owner.begin().await?;
            sqlx::query("ALTER TABLE owner_beta_paper_events DISABLE TRIGGER owner_beta_paper_events_immutable").execute(&mut *tx).await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(corruption.to_string())).execute(&mut *tx).await?;
            denied(exact_query(&mut tx,&transition_request(paper.account,0,"ENABLE",&evidence,"exact-enable")).await,"23514"); tx.rollback().await?;
        }
        let mut tx=db.owner.begin().await?;
        denied(sqlx::query("INSERT INTO owner_beta_paper_events(owner_user_id,account_id,control_epoch,kind,payload,payload_sha256,request_key,request_canonical) VALUES($1,$2,1,'DRAIN_STARTED','{}',daily_0060_digest('{}'),'partial-receipt','{}')").bind(db.owner_id).bind(paper.account).execute(&mut *tx).await,"23514"); tx.rollback().await?;
        let mut down=db.owner.begin().await?;
        denied(sqlx::raw_sql(PAPER_0068_DOWN).execute(&mut *down).await,"23514"); down.rollback().await?;

        let drain=json!({"schema":"owner-beta-paper-drain-v1","expected_epoch":1,"reason_code":"OWNER_STOP"});
        let drained:Value=sqlx::query_scalar("SELECT transition_owner_beta_paper_v1(1,'DRAIN',$1,'exact-drain')").bind(&drain).fetch_one(&db.owner).await?;
        assert_exact_transition(&db,paper.account,1,"DRAIN",&drain,"exact-drain",&drained).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false").execute(&db.owner).await?;
        sqlx::query("UPDATE user_strategy_configs SET is_active=false,daily_enabled=false WHERE id=$1").bind(paper.config).execute(&db.owner).await?;
        sqlx::query("UPDATE data_entitlements SET status='REVOKED'").execute(&db.owner).await?;
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding='{}',daily_cohort='{}'").execute(&db.owner).await?;
        clock.set(initial+300).await?;
        let enable_request=transition_request(paper.account,0,"ENABLE",&evidence,"exact-enable");
        assert_exact_receipt(&db,&enable_request,&enabled).await?;
        let historical=exact_read(&db.owner,&enable_request).await?;
        assert_eq!(historical["current_epoch"],2);
        assert_eq!(historical["event_epoch"],1);
        assert_eq!(historical["current_state"],"DRAINING");
        assert_eq!(historical["result_canonical"],exact_json(&enabled));
        let mut absent=enable_request.clone(); absent["args"]["request_key"]=json!("never-submitted");
        let before=operation_image(&db).await?;
        let unknown=exact_read(&db.owner,&absent).await?;
        assert_eq!(unknown["status"],"ABSENT");
        for key in ["event_id","event_epoch","result_canonical","result_sha256","reset"] {assert_eq!(unknown[key],Value::Null);}
        assert_eq!(before,operation_image(&db).await?);
        Ok(())
    }.await;
    clock.set(0).await?;
    db.close().await?;
    result
}

async fn stop_fixture_in(
    connection: &mut sqlx::PgConnection,
    account: Uuid,
) -> Daily0060Result<Value> {
    stop_fixture_at(connection, account, 0).await
}

async fn stop_fixture_at(
    connection: &mut sqlx::PgConnection,
    account: Uuid,
    expected_epoch: i64,
) -> Daily0060Result<Value> {
    let generation = Uuid::new_v4();
    let intent = sql_operation_intent(account, generation, None, expected_epoch);
    let begun: Value = r5_sql_stage(
        "RESET_BEGIN",
        sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)")
            .bind(expected_epoch)
            .bind(&intent)
            .bind(intent["request_key"].as_str().unwrap())
            .fetch_one(&mut *connection)
            .await,
    )?;
    assert_eq!(begun["outcome"], "OK");
    let fence = begun["control_epoch"]
        .as_i64()
        .ok_or("fixture fence epoch")?;
    assert_eq!(Some(fence), expected_epoch.checked_add(1));
    let reconciled: Value = r5_sql_stage(
        "RESET_RECONCILE",
        sqlx::query_scalar("SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)")
            .bind(generation)
            .bind(fence)
            .bind(hash('b'))
            .bind(format!("{generation}:reconcile:1"))
            .fetch_one(&mut *connection)
            .await,
    )?;
    assert_eq!(reconciled["outcome"], "OK");
    assert_eq!(reconciled["data"]["summary"]["unresolved_commits"], 0);
    assert_eq!(reconciled["data"]["summary"]["live_claims"], 0);
    let now: i64 = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM clock_timestamp())*1000000000)::bigint",
    )
    .fetch_one(&mut *connection)
    .await?;
    let key = format!("{generation}:off");
    let evidence = json!({"schema":"owner-beta-paper-drain-receipt-v1","installation_id":intent["installation_id"],"generation":generation,"account_id":account,"request_key":key,"scope":"PAPER_STOP","expected_epoch":expected_epoch,"resulting_epoch":fence,"intent_sha256":begun["data"]["intent_sha256"],"stage_sha256":hash('c'),"inventory_sha256":hash('b'),"reconciliation_sha256":reconciled["data"]["summary_sha256"],"observation_id":reconciled["data"]["observation_id"],"release_manifest_sha256":hash('a'),"boot_id":Uuid::new_v4(),"observed":{"realtime_ns":now,"monotonic_ns":1,"boottime_ns":1}});
    r5_sql_stage(
        "RESET_OFF",
        sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)")
            .bind(fence)
            .bind(evidence)
            .bind(key)
            .fetch_one(connection)
            .await,
    )
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_reset_races_enable_control_lock() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let caller = login_without_actor(&db, "migration_owner", 1).await?;
    let result: Daily0060Result = async {
        let (paper, evidence) = r5_sql_stage("RESET_SETUP", prepared_paper(&db, false).await)?;
        close_fixture_writer_connections(&db).await?;
        let mut barrier = db.owner.begin().await?;
        r5_sql_stage(
            "RESET_BARRIER",
            sqlx::query("SELECT 1 FROM owner_beta_paper_control FOR UPDATE")
                .execute(&mut *barrier)
                .await,
        )?;
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *barrier)
            .await?;
        let mut waiter = caller.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *waiter)
            .await?;
        assert_ne!(pid, blocker);
        let waiting = tokio::spawn(async move {
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1(0,'ENABLE',$1,'racing-enable')",
            )
            .bind(evidence)
            .fetch_one(&mut *waiter)
            .await
        });
        let blocked = wait_for_pg_blocker(&db, pid, blocker).await;
        if let Err(error) = blocked {
            barrier.rollback().await?;
            let waited = waiting.await?;
            if let Err(wait_error) = waited {
                let _: Daily0060Result<Value> = r5_sql_stage("RESET_ENABLE", Err(wait_error));
            }
            return r5_sql_stage("RESET_WAIT", Err(error));
        }
        let completed_before_unlock = waiting.is_finished();
        emit_r5_paper_test_witness(
            "RESET_WAIT",
            "UPDATE_OK",
            None,
            Some(pid),
            Some(blocker),
            None,
            Some(completed_before_unlock),
        );
        let off = stop_fixture_in(&mut barrier, paper.account).await;
        if off.is_ok() {
            barrier.commit().await?;
        } else {
            barrier.rollback().await?;
        }
        let waited = waiting.await?;
        let off = off?;
        assert!(
            !completed_before_unlock,
            "ENABLE must still wait under the control barrier"
        );
        denied(waited, "40001");
        let reset = r5_sql_stage(
            "RESET_READBACK",
            exact_read(
                &db.owner,
                &exact_request("RESET", Some(paper.account), json!({"expected_epoch":2})),
            )
            .await,
        )?;
        assert_eq!(reset["reset"]["event_id"], off["data"]["event_id"]);
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_events WHERE kind='ENABLED'")
                .fetch_one(&db.owner)
                .await?;
        assert_eq!(count, 0);
        Ok(())
    }
    .await;
    caller.close().await;
    clock.set(0).await?;
    db.close().await?;
    result
}

// SQL-only fixture: all fences, observations, OFF/reset and ENABLE receipts
// come from the public ABI. Synthetic Daily helpers exercise DB joins only;
// this does not prove native ARM origin, process stopping or P4 authority.
async fn reenable_retained_fixture(
    db: &Daily0060Db, clock: &FixtureClock, initial: i64, paper: &SqlPaper, target: Uuid,
) -> Daily0060Result {
    reenable_retained_fixture_with_daily(db, clock, initial, paper, target, false).await?;
    Ok(())
}

async fn reenable_retained_fixture_with_daily(
    db: &Daily0060Db,
    clock: &FixtureClock,
    initial: i64,
    paper: &SqlPaper,
    target: Uuid,
    changed_daily: bool,
) -> Daily0060Result<Value> {
    let mut cohort = paper.cohort.clone();
    let retained_sql = "SELECT jsonb_build_object('target',to_jsonb(t),'lineage',to_jsonb(l),'session',to_jsonb(s),'cash',(SELECT jsonb_agg(to_jsonb(cl) ORDER BY cl.seq) FROM cash_ledger cl WHERE cl.account_id=t.account_id),'orders',(SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM orders o WHERE o.account_id=t.account_id),'fills',(SELECT jsonb_agg(to_jsonb(f) ORDER BY f.id) FROM fills f WHERE f.account_id=t.account_id),'equity',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM daily_equity e WHERE e.account_id=t.account_id)) FROM pending_targets t JOIN owner_beta_paper_lineage l ON l.id=t.beta_lineage_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=t.id WHERE t.id=$1";
    let retained: Value = sqlx::query_scalar(retained_sql)
        .bind(target)
        .fetch_one(&db.owner)
        .await?;
    let (expected_epoch,mut binding):(i64,Value)=sqlx::query_as("SELECT control_epoch,process_binding FROM owner_beta_paper_control WHERE account_id=$1 AND state='ENABLED'")
        .bind(paper.account).fetch_one(&db.owner).await?;
    close_fixture_writer_connections(db).await?;
    let mut tx = db.owner.begin().await?;
    let off = stop_fixture_at(&mut tx, paper.account, expected_epoch).await?;
    assert_eq!(off["outcome"], "OK");
    tx.commit().await?;
    let off_epoch = off["control_epoch"].as_i64().ok_or("fixture OFF epoch")?;
    let reset = exact_read(
        &db.owner,
        &exact_request(
            "RESET",
            Some(paper.account),
            json!({"expected_epoch":off_epoch}),
        ),
    )
    .await?;
    assert_eq!(off["data"]["state"], "OFF");
    assert_eq!(reset["reset"]["kind"], "OFF");
    assert_eq!(reset["reset"]["event_id"], off["data"]["event_id"]);
    assert_eq!(reset["reset"]["control_epoch"], off_epoch);

    // Use three NEW slot/admission/run identities after the committed OFF;
    // no old history or DB creation time is rewritten to satisfy ENABLE.
    let dates: Vec<String> = sqlx::query_scalar(
        "SELECT ($1::text::date+n)::text FROM generate_series(2,4) n ORDER BY n",
    )
    .bind(&paper.witness.session)
    .fetch_all(&db.owner)
    .await?;
    assert_eq!(dates.len(), 3);
    if changed_daily {
        // Real control/config triggers advance epochs; original slots/runs,
        // receipts and financial semantics are untouched. SQL fixture only.
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false WHERE id='etf11'")
            .execute(&db.owner).await?;
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=false WHERE id=$1")
            .bind(paper.config).execute(&db.owner).await?;
        sqlx::query("UPDATE user_strategy_configs SET daily_enabled=true WHERE id=$1")
            .bind(paper.config).execute(&db.owner).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=true,first_auto_session=$1::text::date WHERE id='etf11'")
            .bind(&dates[0]).execute(&db.owner).await?;
        let (activation, config_epoch): (i64, i64) = sqlx::query_as(
            "SELECT d.activation_epoch,s.daily_epoch FROM owner_beta_daily_control d JOIN user_strategy_configs s ON s.id=$1",
        ).bind(paper.config).fetch_one(&db.owner).await?;
        cohort["activation_epoch"] = json!(activation);
        cohort["config_epoch"] = json!(config_epoch);
        cohort["cohort_sha256"] = json!(synthetic_digest(db, "changed-daily-recovery-cohort").await?);
        assert_ne!(cohort["activation_epoch"], paper.cohort["activation_epoch"]);
        assert_ne!(cohort["config_epoch"], paper.cohort["config_epoch"]);
    }

    let days = Value::Array(
        dates
            .iter()
            .enumerate()
            .map(|(i, day)| json!({"date":day,"kind":"TRADING","ordinal":i,"row_hash":hash('4')}))
            .collect(),
    );
    let calendar = synthetic_digest(db, "retained-recovery-post-reset-calendar").await?;
    calendar_claim(
        &mut *db.owner.acquire().await?,
        db.owner_id,
        &calendar,
        paper.cohort["policy_sha256"]
            .as_str()
            .ok_or("fixture policy")?,
        &days,
        false,
    )
    .await?;
    let mut parent: String=sqlx::query_scalar("SELECT COALESCE(s.execution_admission_sha256,sa.admission_sha256) FROM owner_beta_paper_sessions s JOIN owner_beta_paper_source_slots sl ON sl.owner_user_id=s.owner_user_id AND sl.session_date=s.effective_session JOIN owner_beta_paper_source_admissions sa USING(slot_id) WHERE s.pending_target_id=$1")
        .bind(target).fetch_one(&db.owner).await?;
    let mut witnesses = Vec::new();
    for (offset, day) in [2_i64, 3, 4].into_iter().zip(dates) {
        clock
            .set(
                initial
                    .checked_add(offset * 86400)
                    .ok_or("fixture clock overflow")?,
            )
            .await?;
        let witness =
            seed_witness(db, paper.config, &cohort, &day, Some(&parent), false).await?;
        parent = witness.admission.clone();
        witnesses.push(witness);
    }
    let consent: Uuid = sqlx::query_scalar(
        "SELECT consent_event_id FROM owner_beta_paper_control WHERE account_id=$1 AND state='OFF'",
    )
    .bind(paper.account)
    .fetch_one(&db.owner)
    .await?;
    let new_epoch = off_epoch.checked_add(1).ok_or("fixture epoch overflow")?;
    binding["control_epoch"] = json!(new_epoch);
    binding["daily_cohort_sha256"] = cohort["cohort_sha256"].clone();
    binding["consent_event_id"] = json!(consent);
    let process: String = sqlx::query_scalar("SELECT daily_0060_digest($1)")
        .bind(exact_json(&binding))
        .fetch_one(&db.owner)
        .await?;
    let (now,expires):(String,String)=sqlx::query_as("SELECT to_char(clock_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),to_char((clock_timestamp()+interval '120 seconds') AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
        .fetch_one(&db.owner).await?;
    let evidence = json!({"current_cohort":cohort,"process_binding":binding,"process_binding_sha256":process,"evaluated_at":now,"expires_at":expires,"expected_epoch":off_epoch,"new_epoch":new_epoch,
        "session_evidence":witnesses.iter().map(|w|json!({"session_date":w.session,"run_id":w.run,"admission_sha256":w.admission,"source_event_sha256":w.source,"root_event_sha256":w.root})).collect::<Vec<_>>()});
    let enabled: Value = sqlx::query_scalar(
        "SELECT transition_owner_beta_paper_v1($1,'ENABLE',$2,'retained-recovery-enable')",
    )
    .bind(off_epoch)
    .bind(&evidence)
    .fetch_one(&db.owner)
    .await?;
    assert_eq!(enabled["outcome"], "OK");
    assert_eq!(enabled["control_epoch"], new_epoch);
    assert_exact_transition(
        db,
        paper.account,
        off_epoch,
        "ENABLE",
        &evidence,
        "retained-recovery-enable",
        &enabled,
    )
    .await?;
    let current_reset = exact_read(
        &db.owner,
        &exact_request(
            "RESET",
            Some(paper.account),
            json!({"expected_epoch":new_epoch}),
        ),
    )
    .await?;
    assert!(
        reset["reset"] == current_reset["reset"],
        "ENABLE cannot rewrite the committed reset"
    );
    let after: Value = sqlx::query_scalar(retained_sql)
        .bind(target)
        .fetch_one(&db.owner)
        .await?;
    if changed_daily {
        for key in ["target", "lineage", "cash", "orders", "fills", "equity"] {
            assert_eq!(retained[key], after[key], "changed Daily preserves original {key}");
        }
        for key in ["receipt_json", "settlement_sha256", "valuation_sha256", "effective_session", "pending_target_id"] {
            assert_eq!(retained["session"][key], after["session"][key], "changed Daily preserves session {key}");
        }
    } else {
        assert!(
            retained == after,
            "lifecycle preserves original lineage, target, settlement and economics byte-for-byte"
        );
    }
    Ok(cohort)
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_exact_recovery_v1_v2_receipts() -> Daily0060Result {
    let mut first_failure = None;
    for version in [1, 2] {
        let stages = if version == 1 {
            [
                "RECOVERY_V1_SETUP",
                "RECOVERY_V1_TARGET",
                "RECOVERY_V1_E_SOURCE",
                "RECOVERY_V1_SETTLE",
                "RECOVERY_V1_BUILD",
                "RECOVERY_V1_TRANSITION",
                "RECOVERY_V1_READBACK",
            ]
        } else {
            [
                "RECOVERY_V2_SETUP",
                "RECOVERY_V2_TARGET",
                "RECOVERY_V2_E_SOURCE",
                "RECOVERY_V2_SETTLE",
                "RECOVERY_V2_BUILD",
                "RECOVERY_V2_TRANSITION",
                "RECOVERY_V2_READBACK",
            ]
        };
        let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
        let clock = FixtureClock::new().await?;
        let initial = clock.after_cutoff().await?;
        let result: Daily0060Result=async {
            let paper=r5_sql_stage(stages[0],enabled_paper(&db,false).await)?;
            let (target,_)=r5_sql_stage(stages[1],auto_target(&db,&paper,false).await)?;
            clock.set(initial+86400).await?;
            let day:String=sqlx::query_scalar("SELECT effective_session::text FROM owner_beta_paper_sessions WHERE pending_target_id=$1").bind(target).fetch_one(&db.owner).await?;
            let execution=r5_sql_stage(stages[2],seed_witness(&db,paper.config,&paper.cohort,&day,Some(&paper.witness.admission),false).await)?;
            let settlement=r5_sql_stage(stages[3],settlement_packet(&db,&paper,target,&execution.admission,false).await)?;
            assert_eq!(r5_sql_stage(stages[3],settle(&db,&paper,target,&settlement).await)?["outcome"],"OK");
            clock.set(initial+172800).await?;
            let generation=Uuid::new_v4();
            let key=format!("{generation}:recover");
            let evidence_result: Daily0060Result<Value> = async {
            let evidence=if version==1 {
                json!({"schema":"owner-beta-paper-recovery-v1","pending_target_id":target,"effective_session":day,"reason_code":"RECOVERY_REQUIRED","decision_sha256":hash('8')})
            } else {
                reenable_retained_fixture(&db,&clock,initial,&paper,target).await?;
                let (lineage,lineage_epoch,active_epoch,consent,process,signal_admission,execution_admission,original):(Uuid,i64,i64,Uuid,String,String,String,String)=sqlx::query_as("SELECT l.id,l.control_epoch,c.control_epoch,c.consent_event_id,c.process_binding_sha256,l.signal_admission_sha256,s.execution_admission_sha256,s.settlement_sha256 FROM pending_targets t JOIN owner_beta_paper_lineage l ON l.id=t.beta_lineage_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=t.id JOIN owner_beta_paper_control c ON c.account_id=t.account_id WHERE t.id=$1").bind(target).fetch_one(&db.owner).await?;
                assert!(active_epoch>lineage_epoch,"retained recovery uses a genuinely newer SQL-enabled epoch");
                let decision:String=sqlx::query_scalar("SELECT daily_0060_digest(paper_0068_canonical_json(jsonb_build_object('effective_session',$1::text,'pending_target_id',$2::text,'step','VALUE_ONLY')))").bind(&day).bind(target.to_string()).fetch_one(&db.owner).await?;
                let expires:String=sqlx::query_scalar("SELECT to_char((((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1)::timestamp AT TIME ZONE 'Asia/Seoul') AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')").fetch_one(&db.owner).await?;
                json!({"schema":"owner-beta-paper-recovery-v2","installation_id":Uuid::new_v4(),"generation":generation,"account_id":paper.account,"request_key":key,"pending_target_id":target,"lineage_id":lineage,"effective_session":day,"lineage_epoch":lineage_epoch,"active_epoch":active_epoch,"original_receipt_sha256":original,"consent_event_id":consent,"process_config_sha256":process,"step":"VALUE_ONLY","signal_admission_sha256":signal_admission,"execution_admission_sha256":execution_admission,"decision_sha256":decision,"reason_code":"EXPLICIT_RETAINED_RECOVERY","expires_at":expires})
            };
            Ok(evidence)
            }.await;
            let evidence=r5_sql_stage(stages[4],evidence_result)?;
            let expected_epoch:i64=sqlx::query_scalar("SELECT control_epoch FROM owner_beta_paper_control WHERE account_id=$1").bind(paper.account).fetch_one(&db.owner).await?;
            if version==2 {
                for field in ["active_epoch","lineage_epoch"] {
                    let mut invalid=evidence.clone();
                    invalid[field]=if field=="active_epoch" {evidence["lineage_epoch"].clone()} else {json!(expected_epoch)};
                    let before=operation_image(&db).await?;
                    denied(sqlx::query_scalar::<_,Value>("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)").bind(expected_epoch).bind(&invalid).bind(&key).fetch_one(&db.owner).await,"42501");
                    assert!(before==operation_image(&db).await?,"rejected recovery epoch cannot write an event, grant or economics");
                }
            }
            let original:Value=r5_sql_stage(stages[5],sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)").bind(expected_epoch).bind(&evidence).bind(&key).fetch_one(&db.owner).await)?;
            assert_eq!(original["outcome"],"OK");
            r5_sql_stage(stages[6],assert_exact_transition(&db,paper.account,expected_epoch,"RECOVERY",&evidence,&key,&original).await)?;
            if version==2 {
                let count:i64=sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_recovery_grants WHERE request_key=$1").bind(&key).fetch_one(&db.owner).await?;
                assert_eq!(count,1);
                let grant_exact:bool=sqlx::query_scalar("SELECT active_epoch=$2 AND lineage_epoch=$3 AND original_receipt_sha256=$4 AND process_config_sha256=$5 FROM owner_beta_paper_recovery_grants WHERE request_key=$1").bind(&key).bind(expected_epoch).bind(evidence["lineage_epoch"].as_i64().ok_or("original epoch")?).bind(evidence["original_receipt_sha256"].as_str().ok_or("original receipt")?).bind(evidence["process_config_sha256"].as_str().ok_or("active process")?).fetch_one(&db.owner).await?;
                assert!(grant_exact,"grant retains original lineage/receipt separately from current authority");
            }
            Ok(())
        }.await;
        clock.set(0).await?;
        db.close().await?;
        // Keep each version's original positive assertions. A failed v1 must
        // not prevent the separate v2 fixture from yielding its own witness.
        if first_failure.is_none() {
            first_failure = result.err();
        }
    }
    match first_failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
const PAPER_0067_ABI: &str = include_str!("../../../../docs/contracts/owner-beta-paper-sql-v1.md");

async fn preview_packet(
    db: &Daily0060Db,
    f: &SqlPaper,
    mode: &str,
    trade: bool,
) -> Daily0060Result<(Uuid, Uuid, Value)> {
    let request = || {
        if mode == "AUTO" {
            sqlx::query_scalar::<_, Value>("SELECT queue_owner_beta_paper_target_v1($1,1,$2)")
                .bind(f.witness.run)
                .bind(&f.digest)
        } else {
            sqlx::query_scalar::<_, Value>(
                "SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'manual-request',$3)",
            )
            .bind(f.account)
            .bind(f.witness.run)
            .bind(&f.digest)
        }
    };
    let pool = if mode == "AUTO" { &db.worker } else { &db.app };
    let first = request().fetch_one(pool).await?;
    assert!(
        matches!(first["outcome"].as_str(), Some("OK" | "QUEUED" | "REPLAY")),
        "{first}"
    );
    let retry = request().fetch_one(pool).await?;
    assert_eq!(retry["outcome"], "REPLAY");
    assert_eq!(retry["data"]["preview_id"], first["data"]["preview_id"]);
    let preview = Uuid::parse_str(first["data"]["preview_id"].as_str().ok_or("preview")?)?;
    let job = Uuid::parse_str(first["data"]["job_id"].as_str().ok_or("job")?)?;
    // Generic preview claims do not run the predecessor Daily timestamp
    // trigger. Mirror the real queue claim's complete lease/attempt tuple.
    let mut claim = db.worker.begin().await?;
    let attempt_no:i32=sqlx::query_scalar("UPDATE jobs SET status='RUNNING',attempt_count=attempt_count+1,locked_by='sql-preview-worker',locked_at=clock_timestamp(),started_at=COALESCE(started_at,clock_timestamp()) WHERE id=$1 AND owner_user_id=$2 AND job_type='owner_beta_paper_preview' AND status='QUEUED' AND available_at<=clock_timestamp() RETURNING attempt_count").bind(job).bind(db.owner_id).fetch_one(&mut *claim).await?;
    sqlx::query("INSERT INTO job_attempts(job_id,attempt_no,outcome,claimed_by,started_at) VALUES($1,$2,'RUNNING','sql-preview-worker',clock_timestamp())").bind(job).bind(attempt_no).execute(&mut *claim).await?;
    claim.commit().await?;
    let snapshot: Value =
        sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)")
            .bind(preview)
            .bind(job)
            .bind(&f.digest)
            .fetch_one(&db.worker)
            .await?;
    assert_eq!(snapshot["outcome"], "OK");
    let s = &snapshot["data"];
    let held = !s["account_state_preimage"]["positions"]
        .as_object()
        .ok_or("positions")?
        .is_empty();
    if held && !trade {
        assert_eq!(
            s["previous_target_weights"],
            json!([{"instrument_id":"069500.KRX","weight":"1.000000"}]),
            "unchanged preview must use strictly earlier applied allocation"
        );
    }
    let allocation: String = sqlx::query_scalar(
        "SELECT signal_allocation_sha256 FROM paper_rebalance_previews WHERE id=$1",
    )
    .bind(preview)
    .fetch_one(&db.owner)
    .await?;
    assert_eq!(
        s["signal_allocation_sha256"], allocation,
        "snapshot must expose the persisted signal identity"
    );
    let result = json!({"schema":"owner-beta-paper-preview-result-v1","target_snapshot_sha256":s["target_snapshot_sha256"],"signal_allocation_sha256":s["signal_allocation_sha256"],"plan_status":if trade {"REBALANCE"} else {"NO_TRADE"},"no_trade_reasons":if trade {json!([])} else if held {json!(["SIGNAL_UNCHANGED"])} else {json!(["CASH_TARGET"])},"equity":if held {"9999900.0000"} else {"10000000.0000"},"cash_before":s["account_state_preimage"]["cash"],"available_cash":s["account_state_preimage"]["cash"],"leftover_cash":if trade {json!("9898900.0000")} else {s["account_state_preimage"]["cash"].clone()},"buy_notional":if trade {"100100.0000"} else {"0.0000"},"sell_notional":"0.0000","explicit_fees":if trade {"1000.0000"} else {"0.0000"},"informational_slippage":if trade {"100.0000"} else {"0.0000"},"decisions":[],"orders":if trade {json!([{"instrument_id":"069500.KRX","side":"BUY","quantity":"10","raw_price":"10000.0000","estimated_execution_price":"10010.0000","notional":"100100.0000","commission":"1000.0000","tax":"0.0000","informational_slippage":"100.0000"}])} else {json!([])},"cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1,"price_observations":[]});
    let publication = json!({"schema":"owner-beta-paper-preview-v1","preview_id":preview.to_string(),"lineage_id":s["lineage_id"],"apply_mode":mode,"control_epoch":1,"attempt_id":s["attempt_id"],"attempt_no":s["attempt_no"],"worker_id":s["worker_id"],"process_config_sha256":f.digest,"account_state_version":s["account_state_version"],"account_state_sha256":s["account_state_sha256"],"account_state_preimage":s["account_state_preimage"],"cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1,"target_snapshot_sha256":s["target_snapshot_sha256"],"signal_allocation_sha256":s["signal_allocation_sha256"],"preview_token":"e".repeat(64),"result":result});
    Ok((preview, job, publication))
}

async fn record_source_conflict(db: &Daily0060Db, f: &SqlPaper) -> Daily0060Result<String> {
    // Seed only the existing poll receipt/state relation. The actual conflict
    // event and overlapping admission links are written through the 0060 API.
    let (policy, activation): (String, i64) = sqlx::query_as(
        "SELECT policy_sha256,activation_epoch FROM owner_beta_daily_control WHERE id='etf11' AND owner_user_id=$1",
    )
    .bind(db.owner_id)
    .fetch_one(&db.owner)
    .await?;
    let receipt = Uuid::new_v4();
    let mut tx = db.owner.begin().await?;
    sqlx::query("INSERT INTO owner_beta_daily_poll_receipts(receipt_id,stage,owner_user_id,policy_sha256,activation_epoch) VALUES($1,'ADMISSION',$2,$3,$4)")
        .bind(receipt)
        .bind(db.owner_id)
        .bind(policy)
        .bind(activation)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO owner_beta_daily_poll_state(stage,next_poll_at,latest_receipt_id) VALUES('ADMISSION',clock_timestamp(),$1) ON CONFLICT(stage) DO UPDATE SET next_poll_at=EXCLUDED.next_poll_at,latest_receipt_id=EXCLUDED.latest_receipt_id")
        .bind(receipt)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let event = json!({
        "source_kind":"BAR",
        "range_start":f.witness.session,
        "range_end":f.witness.session,
        "code":"SOURCE_CONFLICT",
        "evidence_sha256":hash('f')
    });
    let outcome: (String, Option<String>, String, String) = sqlx::query_as(
        "SELECT disposition,admission_sha256,reason_code,event_sha256 FROM block_owner_beta_daily_input($1,$2)",
    )
    .bind(receipt)
    .bind(event)
    .fetch_one(&db.owner)
    .await?;
    assert_eq!(outcome.0, "BLOCKED");
    assert_eq!(outcome.2, "SOURCE_CONFLICT");
    Ok(outcome.3)
}

async fn call_serving_preview(
    connection: &mut sqlx::PgConnection,
    operation: &str,
    preview: Uuid,
    job: Uuid,
    packet: &Value,
    digest: &str,
) -> Result<Value, sqlx::Error> {
    match operation {
        "snapshot" => {
            sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)")
                .bind(preview)
                .bind(job)
                .bind(digest)
                .fetch_one(&mut *connection)
                .await
        }
        "publish" => {
            sqlx::query_scalar("SELECT publish_owner_beta_paper_preview_v1($1,$2,$3,$4)")
                .bind(preview)
                .bind(job)
                .bind(packet)
                .bind(digest)
                .fetch_one(&mut *connection)
                .await
        }
        "fail" => {
            let error = json!({
                "code":"SYNTHETIC_FAILURE",
                "message":"SQL lock-order test failure",
                "attempt_id":packet["attempt_id"],
                "attempt_no":packet["attempt_no"],
                "worker_id":packet["worker_id"],
                "process_config_sha256":digest
            });
            sqlx::query_scalar("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)")
                .bind(preview)
                .bind(job)
                .bind(error)
                .bind(digest)
                .fetch_one(&mut *connection)
                .await
        }
        _ => Err(sqlx::Error::Protocol(
            "unknown preview lock-order operation".into(),
        )),
    }
}

async fn terminal_serving_lock_order_case(
    operation: &'static str,
    actor_before: Option<String>,
    terminal_actor: Option<String>,
) -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, packet) = preview_packet(&db, &f, "MANUAL", false).await?;

        // B owns the exact job row before A enters a serving routine.
        let mut terminal = db.worker.begin().await?;
        let terminal_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *terminal)
            .await?;
        let _: Uuid = sqlx::query_scalar("SELECT id FROM jobs WHERE id=$1 FOR UPDATE")
            .bind(job)
            .fetch_one(&mut *terminal)
            .await?;
        if let Some(actor) = terminal_actor.as_deref() {
            sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
                .bind(actor)
                .execute(&mut *terminal)
                .await?;
        }
        let terminal_actor_before: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *terminal)
                .await?;

        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let worker_pool = db.worker.clone();
        let packet_for_call = packet.clone();
        let digest_for_call = f.digest.clone();
        let actor_for_call = actor_before.clone();
        let serving = tokio::spawn(async move {
            let mut connection = worker_pool.acquire().await?;
            if let Some(actor) = actor_for_call.as_deref() {
                sqlx::query("SELECT set_config('app.actor_user_id',$1,false)")
                    .bind(actor)
                    .execute(&mut *connection)
                    .await?;
            }
            let actor_before_call: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *connection)
                    .await?;
            let serving_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            let _ = pid_tx.send(serving_pid);
            let call = call_serving_preview(
                &mut connection,
                operation,
                preview,
                job,
                &packet_for_call,
                &digest_for_call,
            )
            .await;
            let actor_after_call: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *connection)
                    .await?;
            sqlx::query("RESET app.actor_user_id")
                .execute(&mut *connection)
                .await?;
            Ok::<_, sqlx::Error>((call, actor_before_call, actor_after_call))
        });

        let serving_pid = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            pid_rx,
        )
        .await
        {
            Ok(Ok(pid)) => Some(pid),
            _ => None,
        };
        let wait_observation = if let Some(pid) = serving_pid {
            wait_for_pg_blocker(&db, pid, terminal_pid)
                .await
                .map_err(|error| error.to_string())
        } else {
            Err("serving backend PID was not reported".to_owned())
        };
        let preview_lock_probe = if wait_observation.is_ok() {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM paper_rebalance_previews WHERE id=$1 FOR UPDATE NOWAIT",
            )
            .bind(preview)
            .fetch_one(&db.owner)
            .await
            .map(|_| ())
            .map_err(|_| "serving backend held the preview while waiting on the job".to_owned())
        } else {
            Err("skipped preview lock probe because the job wait was not observed".to_owned())
        };

        let transition = if wait_observation.is_ok() && preview_lock_probe.is_ok() {
            async {
                let changed = sqlx::query("UPDATE job_attempts SET outcome='FAILED',finished_at=clock_timestamp() WHERE job_id=$1 AND outcome='RUNNING'")
                    .bind(job)
                    .execute(&mut *terminal)
                    .await?;
                if changed.rows_affected() != 1 {
                    return Err(sqlx::Error::Protocol(
                        "current attempt was not transitioned".into(),
                    ));
                }
                let changed = sqlx::query("UPDATE jobs SET status='FAILED',error_code='attempts_exhausted',locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp() WHERE id=$1 AND status='RUNNING'")
                    .bind(job)
                    .execute(&mut *terminal)
                    .await?;
                if changed.rows_affected() != 1 {
                    return Err(sqlx::Error::Protocol(
                        "current job was not transitioned".into(),
                    ));
                }
                let actor: Option<String> = sqlx::query_scalar(
                    "SELECT current_setting('app.actor_user_id',true)",
                )
                .fetch_one(&mut *terminal)
                .await?;
                Ok::<_, sqlx::Error>(actor)
            }
            .await
        } else {
            Err(sqlx::Error::Protocol(
                "lock-order observations failed".into(),
            ))
        };
        let terminal_actor_after = match transition {
            Ok(actor) => terminal.commit().await.map(|_| actor),
            Err(error) => {
                let rolled_back = terminal.rollback().await;
                match rolled_back {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(rollback_error),
                }
            }
        };

        let mut serving = serving;
        let serving_result = match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            &mut serving,
        )
        .await
        {
            Ok(Ok(result)) => Some(result),
            Ok(Err(_)) => None,
            Err(_) => {
                // B has already committed or rolled back, so abort is only a
                // bounded cleanup fallback; always reap the task before exit.
                serving.abort();
                let _ = serving.await;
                None
            }
        };

        if let Err(error) = wait_observation {
            return Err(error.into());
        }
        if let Err(error) = preview_lock_probe {
            return Err(error.into());
        }
        let terminal_actor_after = terminal_actor_after?;
        let serving_result = serving_result.ok_or("serving task did not complete")??;
        assert_same_actor(&terminal_actor_before, &terminal_actor_after);
        assert_same_actor(&serving_result.1, &serving_result.2);
        match operation {
            "snapshot" => assert_eq!(
                serving_result
                    .0
                    .as_ref()
                    .err()
                    .and_then(sqlx::Error::as_database_error)
                    .and_then(|error| error.code())
                    .as_deref(),
                Some("42501"),
                "snapshot preserves its typed lost-claim refusal"
            ),
            "publish" | "fail" => {
                assert_eq!(serving_result.0.as_ref().unwrap()["outcome"], "STALE")
            }
            _ => return Err("unknown serving lock-order operation".into()),
        }
        let projected: (String, Value, bool) = sqlx::query_as(
            "SELECT status,error_json,completed_at IS NOT NULL AND completed_at<=clock_timestamp() FROM paper_rebalance_previews WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
        )
        .bind(preview)
        .bind(db.owner_id)
        .bind(job)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(projected.0, "FAILED");
        assert_eq!(projected.1["code"], "OWNER_BETA_PAPER_PREVIEW_ATTEMPTS_EXHAUSTED");
        assert!(projected.2, "projection uses the actual database clock");
        let economics: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(economics, (0, 0));
        Ok(())
    }
    .await;
    let close = db.close().await;
    let reset_clock = clock.set(0).await;
    close?;
    reset_clock?;
    result
}

async fn publish_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &SqlPaper,
    preview: Uuid,
    job: Uuid,
    packet: &Value,
) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT publish_owner_beta_paper_preview_v1($1,$2,$3,$4)")
        .bind(preview)
        .bind(job)
        .bind(packet)
        .bind(&f.digest)
        .fetch_one(&mut **tx)
        .await
}
async fn finish_preview(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: Uuid,
) -> Daily0060Result {
    sqlx::query("UPDATE jobs SET status='SUCCEEDED',locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp() WHERE id=$1").bind(job).execute(&mut **tx).await?;
    sqlx::query("UPDATE job_attempts SET outcome='SUCCEEDED',finished_at=clock_timestamp() WHERE job_id=$1 AND outcome='RUNNING'").bind(job).execute(&mut **tx).await?;
    Ok(())
}

async fn manual_ready_auto_running(
    db: &Daily0060Db,
    f: &SqlPaper,
) -> Daily0060Result<(Uuid, Uuid, Value, Uuid, Uuid, Value)> {
    let (manual, manual_job, manual_packet) = preview_packet(db, f, "MANUAL", false).await?;
    let mut tx = db.worker.begin().await?;
    assert_eq!(
        publish_in(&mut tx, f, manual, manual_job, &manual_packet).await?["outcome"],
        "OK"
    );
    finish_preview(&mut tx, manual_job).await?;
    tx.commit().await?;
    let (automatic, automatic_job, automatic_packet) = preview_packet(db, f, "AUTO", false).await?;
    Ok((
        manual,
        manual_job,
        manual_packet,
        automatic,
        automatic_job,
        automatic_packet,
    ))
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_enable_auto_publication_apply_and_revalidation() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,true).await?;
        let (preview,job,packet)=preview_packet(&db,&f,"AUTO",true).await?;
        let mut rolled=db.worker.begin().await?;
        let tentative=publish_in(&mut rolled,&f,preview,job,&packet).await?;assert_eq!(tentative["outcome"],"OK");
        rolled.rollback().await?;
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM pending_targets WHERE account_id=$1").bind(f.account).fetch_one(&db.owner).await?,0,"publication rollback must remove AUTO target/session");
        let mut tx=db.worker.begin().await?;
        let applied=publish_in(&mut tx,&f,preview,job,&packet).await?;assert_eq!(applied["outcome"],"OK");
        finish_preview(&mut tx,job).await?;tx.commit().await?;
        let target=Uuid::parse_str(applied["data"]["pending_target_id"].as_str().ok_or("AUTO target")?)?;
        denied(sqlx::query("SELECT preflight_paper_target($1,$2)").bind(target).bind(db.owner_id).execute(&db.worker).await,"42501");
        let persisted:(String,String,String,i64)=sqlx::query_as("SELECT p.status,t.source_kind,s.processing_origin,(SELECT count(*) FROM orders WHERE account_id=t.account_id) FROM paper_rebalance_previews p JOIN pending_targets t ON t.id=p.pending_target_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=t.id WHERE p.id=$1").bind(preview).fetch_one(&db.owner).await?;
        assert_eq!(persisted,("APPLIED".into(),"OWNER_BETA_DAILY_AUTO".into(),"AUTO".into(),0));
        let replay:Value=sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)").bind(preview).bind(&f.digest).fetch_one(&db.worker).await?;assert_eq!(replay["outcome"],"REPLAY");
        denied(sqlx::query("SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)").bind(preview).bind(&f.digest).execute(&db.app).await,"42501");
        let before:(String,i64,i64)=sqlx::query_as("SELECT state,control_epoch,(SELECT count(*) FROM owner_beta_paper_events) FROM owner_beta_paper_control").fetch_one(&db.owner).await?;
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        let blocked:Value=sqlx::query_scalar("SELECT read_owner_beta_paper_state_v1()").fetch_one(&db.app).await?;assert_eq!(blocked["data"]["control"]["state"],"BLOCKED");
        denied(sqlx::query("SELECT snapshot_owner_beta_paper_target_v1($1,1,$2)").bind(target).bind(&f.digest).execute(&db.worker).await,"42501");
        let after:(String,i64,i64)=sqlx::query_as("SELECT state,control_epoch,(SELECT count(*) FROM owner_beta_paper_events) FROM owner_beta_paper_control").fetch_one(&db.owner).await?;assert_eq!(before,after,"derived BLOCKED must not mutate authority/history");
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[test]
fn paper_0067_source_contract_has_snapshot_fingerprint_and_no_trade_path() {
    let target_snapshot = PAPER_0067_UP
        .split("CREATE FUNCTION public.snapshot_owner_beta_paper_target_v1")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("target snapshot function source");
    assert!(
        target_snapshot
            .contains("state_hash:=public.paper_0067_account_state_sha256(ver,cash,pos)")
    );
    assert!(target_snapshot.contains("'account_state_sha256',state_hash"));
    assert!(target_snapshot.contains("'initial_account_state_sha256',s.account_state_sha256"));
    assert!(target_snapshot.contains("'settlement_sha256',s.settlement_sha256"));
    assert!(target_snapshot.contains("'ledger_after_sha256',s.ledger_after_sha256"));
    assert!(target_snapshot.contains("'ledger_after_version',s.ledger_after_version"));
    assert!(target_snapshot.contains("'valuation_sha256',s.valuation_sha256"));
    assert!(target_snapshot.contains("'receipt_json',s.receipt_json"));
    assert!(target_snapshot.contains("prior_session.effective_session<l.effective_session"));
    assert!(!target_snapshot.contains("'account_state_sha256',s.account_state_sha256"));

    let submit = PAPER_0067_UP
        .split("CREATE FUNCTION public.submit_owner_beta_paper_preview_v1")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("submit function source");
    let auto_queue = PAPER_0067_UP
        .split("CREATE FUNCTION public.queue_owner_beta_paper_target_v1")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("auto queue function source");
    assert!(!submit.contains("UNCHANGED_SIGNAL"));
    assert!(!auto_queue.contains("UNCHANGED_SIGNAL"));
    let apply_target = PAPER_0067_UP
        .split("CREATE FUNCTION public.paper_0067_apply_target")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("shared target application function source");
    let apply_lock = apply_target
        .find("public.daily_0060_source_lock(c.owner_user_id,false)")
        .expect("shared source lock before target admission recheck");
    let apply_usable = apply_target
        .find("public.daily_0060_admission_usable(d,l.signal_admission_sha256,false)")
        .expect("target signal admission usability recheck");
    let apply_insert = apply_target
        .find("INSERT INTO public.pending_targets")
        .expect("target write after admission recheck");
    assert!(apply_lock < apply_usable && apply_usable < apply_insert);
    assert!(
        apply_target.contains(
            "owner_beta_daily_control WHERE id='etf11' AND owner_user_id=c.owner_user_id"
        )
    );
    assert!(PAPER_0067_UP.contains("paper_0067_apply_target(l.id,'MANUAL'"));
    assert!(PAPER_0067_UP.contains("paper_0067_apply_target(l.id,'AUTO'"));
    let settlement = PAPER_0067_UP
        .split("CREATE FUNCTION public.commit_owner_beta_paper_settlement_v1")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("settlement function source");
    let settlement_lock = settlement
        .find("public.daily_0060_source_lock(c.owner_user_id,false)")
        .expect("settlement source fence");
    let settlement_usable = settlement
        .find("public.daily_0060_admission_usable(daily,l.signal_admission_sha256,false)")
        .expect("settlement signal usability check");
    assert!(settlement_lock < settlement_usable);
    let valuation = PAPER_0067_UP
        .split("CREATE FUNCTION public.commit_owner_beta_paper_valuation_v1")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("valuation function source");
    let valuation_lock = valuation
        .find("public.daily_0060_source_lock(c.owner_user_id,false)")
        .expect("valuation source fence");
    let valuation_usable = valuation
        .find(
            "public.daily_0060_admission_usable(d,valuation->>'valuation_admission_sha256',false)",
        )
        .expect("valuation admission usability check");
    assert!(valuation_lock < valuation_usable);
    assert!(submit.contains("prior_lineage public.owner_beta_paper_lineage"));
    assert!(submit.contains("FOR UPDATE;\n  IF FOUND THEN"));
    for field in [
        "prior_lineage.factor_snapshot_sha256 IS DISTINCT FROM r.factor_snapshot_sha256",
        "prior_lineage.target_weights_json IS DISTINCT FROM arr",
        "prior_lineage.signal_admission_sha256 IS DISTINCT FROM adm.admission_sha256",
        "prior_lineage.calendar_evidence_sha256 IS DISTINCT FROM e.calendar_evidence_sha256",
    ] {
        assert!(
            submit.contains(field),
            "manual lineage reuse must check {field}"
        );
    }
    let terminal_projection = PAPER_0067_UP
        .split("CREATE FUNCTION public.paper_0067_project_terminal_preview()")
        .nth(1)
        .and_then(|tail| tail.split("END $$;").next())
        .expect("terminal job projection function source");
    assert!(PAPER_0067_UP.contains("AFTER UPDATE OF status ON public.jobs"));
    assert!(
        terminal_projection
            .contains("TG_TABLE_SCHEMA<>'public' OR TG_TABLE_NAME<>'jobs' OR TG_OP<>'UPDATE'")
    );
    assert!(terminal_projection.contains("p.owner_user_id=NEW.owner_user_id"));
    assert!(terminal_projection.contains("p.beta_apply_mode=(NEW.payload_json->>'apply_mode')"));
    assert!(
        terminal_projection
            .contains("p.input_kind='OWNER_BETA_DAILY_V1' AND p.status IN ('PENDING','RUNNING')")
    );
    assert!(terminal_projection.contains("OWNER_BETA_PAPER_PREVIEW_ATTEMPTS_EXHAUSTED"));
    assert!(terminal_projection.contains("OWNER_BETA_PAPER_PREVIEW_FAILED"));
    assert!(terminal_projection.contains("OWNER_BETA_PAPER_PREVIEW_CANCELED"));
    assert!(PAPER_0067_UP.contains(
        "NEW.job_type IN ('owner_beta_price_only_simulation','owner_beta_paper_preview')"
    ));
    assert!(terminal_projection.contains("SET search_path=pg_catalog,pg_temp"));
    assert!(terminal_projection.contains("SET lock_timeout='5s' SET statement_timeout='15s'"));
    assert!(!terminal_projection.contains("set_config('app.actor_user_id'"));
    assert!(PAPER_0067_UP.contains("('paper_0067_project_terminal_preview()')"));

    for (name, parameter) in [
        (
            "snapshot_owner_beta_paper_preview_v1",
            "snapshot_owner_beta_paper_preview_v1",
        ),
        (
            "publish_owner_beta_paper_preview_v1",
            "publish_owner_beta_paper_preview_v1",
        ),
        (
            "fail_owner_beta_paper_preview_v1",
            "fail_owner_beta_paper_preview_v1",
        ),
    ] {
        let source = PAPER_0067_UP
            .split(&format!("CREATE FUNCTION public.{name}"))
            .nth(1)
            .and_then(|tail| tail.split("END $$;").next())
            .expect("serving preview function source");
        let authority = source
            .find("public.paper_0067_authority('PREVIEW'")
            .expect("unlocked preview selector remains bound to authority");
        let job_lock = source
            .find(&format!(
                "FROM public.jobs WHERE id={parameter}.job_id AND owner_user_id=c.owner_user_id AND job_type='owner_beta_paper_preview' AND status='RUNNING'"
            ))
            .expect("explicit job argument is locked before the mutable preview");
        let saved_job_claim = source
            .find("job_claimed:=FOUND")
            .expect("job lock outcome is retained across the preview selector");
        let preview_lock = source
            .find("FROM public.paper_rebalance_previews WHERE id=")
            .expect("mutable preview row lock");
        let association = source
            .find(&format!("p.job_id<>{parameter}.job_id"))
            .expect("preview/job association is rechecked after both locks");
        assert!(
            authority < job_lock
                && job_lock < saved_job_claim
                && saved_job_claim < preview_lock
                && preview_lock < association,
            "{name} must retain authority binding and lock jobs before preview"
        );
    }

    assert!(PAPER_0067_DOWN.contains("IF NEW.job_type = 'owner_beta_price_only_simulation' THEN"));
    assert!(PAPER_0067_UP.contains("'SETTLED_NO_TRADE'"));
    assert!(PAPER_0067_UP.contains("public.paper_0067_generic_enqueue_outbox"));
    assert!(PAPER_0067_UP.contains("INSERT INTO public.daily_equity"));
    assert!(!PAPER_0067_UP.contains("jsonb_object_length("));

    for field in [
        "initial_account_state_sha256",
        "settlement_sha256",
        "ledger_after_sha256",
        "ledger_after_version",
        "valuation_sha256",
        "receipt_json",
    ] {
        assert!(PAPER_0067_ABI.contains(field), "ABI missing {field}");
    }
    let precheck = PAPER_0067_DOWN.find("DO $$ BEGIN").expect("down precheck");
    let first_drop = [
        PAPER_0067_DOWN.find("DROP TRIGGER"),
        PAPER_0067_DOWN.find("DROP FUNCTION"),
        PAPER_0067_DOWN.find("DROP TABLE"),
    ]
    .into_iter()
    .flatten()
    .min()
    .expect("down destructive DDL");
    assert!(
        precheck < first_drop,
        "global history precheck must precede DDL"
    );
    let abi_lineage = PAPER_0067_ABI
        .find("AUTO queue retries return the same preview/job/lineage")
        .expect("ABI lineage contract");
    assert!(PAPER_0067_ABI[abi_lineage..].contains("same immutable lineage"));
    assert!(PAPER_0067_ABI.contains("daily_0060_source_lock"));
    assert!(PAPER_0067_ABI.contains("OWNER_BETA_PAPER_PREVIEW_ATTEMPTS_EXHAUSTED"));
}

fn require_r5_03_database_url() -> Daily0060Result<String> {
    let nonce = env::var("R5_03_ISOLATED_NONCE")?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("R5-03 isolated nonce must be 32 hexadecimal characters".into());
    }
    if env::var("R5_03_FIXTURE_CLOCK_FILE")? != "/tmp/r5-03-clock/offset" {
        return Err("R5-03 fixture clock path does not match the isolated runner".into());
    }
    let clock_path = std::path::Path::new("/tmp/r5-03-clock/offset");
    let metadata = std::fs::symlink_metadata(clock_path)?;
    if !metadata.file_type().is_file() || std::fs::read(clock_path)? != b"+0\n" {
        return Err("R5-03 fixture clock must be a regular file initialized to +0".into());
    }
    let url = env::var("DATABASE_URL")?;
    if !url.contains(":55438/") {
        return Err(
            "R5-03 DATABASE_URL must target the isolated loopback PostgreSQL port 55438".into(),
        );
    }
    Ok(url)
}

async fn verify_r5_03_postgres_identity(pool: &PgPool) -> Daily0060Result<()> {
    let (address, port, version, database, has_system_id): (
        Option<String>,
        Option<i32>,
        i32,
        String,
        bool,
    ) = sqlx::query_as(
        "SELECT host(inet_server_addr()),inet_server_port(),current_setting('server_version_num')::int,current_database(),system_identifier IS NOT NULL FROM pg_control_system()",
    )
    .fetch_one(pool)
    .await?;
    if !matches!(address.as_deref(), Some("127.0.0.1" | "::1"))
        || port != Some(55438)
        || version / 10000 != 18
        || database == "production"
        || !has_system_id
    {
        return Err(format!(
            "R5-03 database identity mismatch: addr={address:?}, port={port:?}, version={version}, database={database:?}, system_identifier_present={has_system_id}"
        )
        .into());
    }
    Ok(())
}

async fn paper_catalog(connection: &mut sqlx::PgConnection) -> Daily0060Result<serde_json::Value> {
    Ok(sqlx::query_scalar(
        r#"WITH tables AS (
            SELECT c.* FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='public' AND c.relname=ANY(ARRAY[
                'accounts','user_strategy_configs','owner_beta_daily_admissions',
                'account_strategy_bindings','cash_ledger','positions','orders','fills','daily_equity',
                'jobs','pending_targets','paper_rebalance_previews','owner_beta_paper_control',
                'owner_beta_paper_events','owner_beta_paper_lineage','owner_beta_paper_sessions',
                'owner_beta_paper_session_events','owner_beta_paper_operations',
                'owner_beta_paper_operation_observations','owner_beta_paper_recovery_grants'])
        ), functions AS (
            SELECT p.* FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
            WHERE n.nspname='public' AND (
                p.proname LIKE 'paper_0067_%' OR p.proname LIKE 'paper_0068_%' OR p.proname LIKE '%owner_beta_paper%'
                OR p.proname=ANY(ARRAY['lock_paper_rebalance_preview_submission',
                'snapshot_paper_rebalance_preview','publish_paper_rebalance_preview',
                'fail_paper_rebalance_preview','apply_paper_rebalance_preview',
                'queue_scheduled_paper_targets','preflight_paper_target','enqueue_paper_settlement_outbox',
                'sync_recommendation_run_from_terminal_job']))
        ) SELECT jsonb_build_object(
            'tables',COALESCE((SELECT jsonb_agg(jsonb_build_array(t.relname,r.rolname,t.relrowsecurity,t.relforcerowsecurity) ORDER BY t.relname) FROM tables t JOIN pg_roles r ON r.oid=t.relowner),'[]'),
            'columns',COALESCE((SELECT jsonb_agg(jsonb_build_array(t.relname,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,pg_get_expr(d.adbin,d.adrelid)) ORDER BY t.relname,a.attnum) FROM tables t JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum>0 AND NOT a.attisdropped LEFT JOIN pg_attrdef d ON d.adrelid=t.oid AND d.adnum=a.attnum),'[]'),
            'constraints',COALESCE((SELECT jsonb_agg(jsonb_build_array(t.relname,c.conname,pg_get_constraintdef(c.oid),c.convalidated,c.condeferrable,c.condeferred) ORDER BY t.relname,c.conname) FROM tables t JOIN pg_constraint c ON c.conrelid=t.oid),'[]'),
            'indexes',COALESCE((SELECT jsonb_agg(pg_get_indexdef(i.indexrelid) ORDER BY t.relname,c.relname) FROM tables t JOIN pg_index i ON i.indrelid=t.oid JOIN pg_class c ON c.oid=i.indexrelid),'[]'),
            'policies',COALESCE((SELECT jsonb_agg(jsonb_build_array(t.relname,p.polname,p.polcmd,p.polpermissive,(SELECT jsonb_agg(CASE WHEN role_id=0 THEN 'PUBLIC' ELSE pg_get_userbyid(role_id)::text END ORDER BY role_id) FROM unnest(p.polroles) role_id),pg_get_expr(p.polqual,p.polrelid),pg_get_expr(p.polwithcheck,p.polrelid)) ORDER BY t.relname,p.polname) FROM tables t JOIN pg_policy p ON p.polrelid=t.oid),'[]'),
            'triggers',COALESCE((SELECT jsonb_agg(pg_get_triggerdef(g.oid) ORDER BY t.relname,g.tgname) FROM tables t JOIN pg_trigger g ON g.tgrelid=t.oid AND NOT g.tgisinternal),'[]'),
            'functions',COALESCE((SELECT jsonb_agg(jsonb_build_array(p.proname,pg_get_function_identity_arguments(p.oid),r.rolname,pg_get_functiondef(p.oid),(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(a.grantor),CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee)::text END,a.privilege_type,a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type,a.is_grantable) FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a)) ORDER BY p.proname,pg_get_function_identity_arguments(p.oid)) FROM functions p JOIN pg_roles r ON r.oid=p.proowner),'[]'))"#,
    )
    .fetch_one(connection)
    .await?)
}

fn catalog_difference(
    section: &str,
    before: &serde_json::Value,
    after: &serde_json::Value,
) -> String {
    use std::collections::{BTreeMap, BTreeSet};
    let compact =
        |value: &serde_json::Value| value.to_string().chars().take(240).collect::<String>();
    let a = before.as_array().expect("catalog array");
    let b = after.as_array().expect("catalog array");
    if section == "functions" {
        let keyed = |rows: &[serde_json::Value]| {
            rows.iter()
                .map(|row| (format!("{}({})", row[0], row[1]), row.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let a = keyed(a);
        let b = keyed(b);
        let keys = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
        let mut details = Vec::new();
        for key in keys {
            if a.get(key) == b.get(key) {
                continue;
            }
            match (a.get(key), b.get(key)) {
                (Some(left), Some(right)) => {
                    for (field, label) in [(2, "owner"), (3, "definition"), (4, "acl")] {
                        if left[field] == right[field] {
                            continue;
                        }
                        if field == 3 {
                            let left = left[field]
                                .as_str()
                                .expect("definition")
                                .lines()
                                .collect::<Vec<_>>();
                            let right = right[field]
                                .as_str()
                                .expect("definition")
                                .lines()
                                .collect::<Vec<_>>();
                            let line = (0..left.len().max(right.len()))
                                .find(|i| left.get(*i) != right.get(*i))
                                .expect("changed definition");
                            details.push(format!(
                                "{key} {label} line {}: {:?} -> {:?}",
                                line + 1,
                                left.get(line)
                                    .map(|s| s.chars().take(180).collect::<String>()),
                                right
                                    .get(line)
                                    .map(|s| s.chars().take(180).collect::<String>())
                            ));
                        } else {
                            details.push(format!(
                                "{key} {label}: {} -> {}",
                                compact(&left[field]),
                                compact(&right[field])
                            ));
                        }
                    }
                }
                _ => details.push(format!(
                    "{key}: missing {}",
                    if a.contains_key(key) {
                        "after down"
                    } else {
                        "before up"
                    }
                )),
            }
            if details.len() >= 4 {
                break;
            }
        }
        return details.join("; ");
    }
    let row = (0..a.len().max(b.len()))
        .find(|i| a.get(*i) != b.get(*i))
        .expect("changed catalog");
    format!(
        "{section} rows {} -> {}; first changed row {row}: {} -> {}",
        a.len(),
        b.len(),
        a.get(row).map(&compact).unwrap_or_default(),
        b.get(row).map(&compact).unwrap_or_default()
    )
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest on loopback:55438"]
async fn paper_0067_catalog_reapply_empty_down() -> Daily0060Result {
    let url = require_r5_03_database_url()?;
    let admin = admin_pool(&url)
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    verify_r5_03_postgres_identity(&admin).await?;
    drop(admin);

    let db = Daily0060Db::new(url.clone(), 66).await?;
    let result: Daily0060Result = async {
        let before = paper_catalog(&mut *db.owner.acquire().await?).await?;
        MIGRATOR.run(&db.owner).await?;
        MIGRATOR.run(&db.owner).await?;

        let table_count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner WHERE n.nspname='public' AND c.relname=ANY(ARRAY['owner_beta_paper_control','owner_beta_paper_events','owner_beta_paper_lineage','owner_beta_paper_sessions','owner_beta_paper_session_events','owner_beta_paper_operations','owner_beta_paper_operation_observations','owner_beta_paper_recovery_grants']) AND r.rolname='migration_owner' AND c.relrowsecurity AND c.relforcerowsecurity",
        )
        .fetch_one(&db.owner)
        .await?;
        if table_count != 8 {
            return Err("all five beta ledger tables and three 0068 journals must be migration-owned and FORCE RLS".into());
        }

        let privileges: (bool, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
            "SELECT has_function_privilege('app','public.read_owner_beta_paper_state_v1()','EXECUTE'),has_function_privilege('app','public.apply_owner_beta_paper_preview_v1(uuid,text,bigint,text)','EXECUTE'),has_function_privilege('app','public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)','EXECUTE'),has_function_privilege('app','public.commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text)','EXECUTE'),has_function_privilege('worker','public.commit_owner_beta_paper_settlement_v1(uuid,bigint,jsonb,text)','EXECUTE'),has_function_privilege('worker','public.apply_owner_beta_paper_preview_v1(uuid,text,bigint,text)','EXECUTE'),has_function_privilege('migration_owner','public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)','EXECUTE')",
        )
        .fetch_one(&db.owner)
        .await?;
        if privileges != (true, true, false, false, true, false, true) {
            return Err("0067 public function grants do not match the app/worker/migration-owner ABI".into());
        }
        let trigger_privileges:(bool,bool)=sqlx::query_as(
            "SELECT has_function_privilege('app','public.paper_0067_project_terminal_preview()','EXECUTE'),has_function_privilege('worker','public.paper_0067_project_terminal_preview()','EXECUTE')",
        ).fetch_one(&db.owner).await?;
        if trigger_privileges!=(false,false) {
            return Err("terminal projection trigger function must have no app/worker EXECUTE grant".into());
        }

        let maintenance_privileges: (bool,bool,bool,bool,bool,bool) = sqlx::query_as(
            "SELECT has_function_privilege('app','public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text)','EXECUTE'),has_function_privilege('worker','public.inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)','EXECUTE'),has_function_privilege('app','public.reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text)','EXECUTE'),has_function_privilege('migration_owner','public.begin_owner_beta_paper_operation_v1(bigint,jsonb,text)','EXECUTE'),has_function_privilege('migration_owner','public.inspect_owner_beta_paper_operation_v1(uuid,uuid,integer)','EXECUTE'),has_function_privilege('migration_owner','public.reconcile_owner_beta_paper_operation_v1(uuid,bigint,text,text)','EXECUTE')",
        ).fetch_one(&db.owner).await?;
        if maintenance_privileges != (false,false,false,true,true,true) {
            return Err("0068 maintenance functions must be executable only by direct migration_owner".into());
        }

        let mut tx = db.owner.begin().await?;
        sqlx::raw_sql(PAPER_0068_DOWN).execute(&mut *tx).await?;
        sqlx::raw_sql(PAPER_0067_DOWN).execute(&mut *tx).await?;
        let after = paper_catalog(&mut tx).await?;
        for section in ["tables", "columns", "constraints", "indexes", "policies", "triggers", "functions"] {
            if before[section] != after[section] {
                return Err(format!("empty down must restore {section}: {}",catalog_difference(section,&before[section],&after[section])).into());
            }
        }
        tx.rollback().await?;
        Ok(())
    }
    .await;
    let cleanup = db.close().await;
    cleanup?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0067_0068_fence_reconcile_offline_readback_and_history_guard() -> Daily0060Result {
    let url = require_r5_03_database_url()?;
    let db = Daily0060Db::new(url, 68).await?;
    let caller = login_without_actor(&db, "migration_owner", 2).await?;
    let result: Daily0060Result = async {
        let (account, config, configured) = configure(&db).await?;
        let configure_request = exact_request("CONFIGURE",None,json!({"strategy_config_id":config,"account_name":"SQL fixture","contract_canonical":exact_json(&financial_contract()),"process_binding":null,"request_key":"sql-config"}));
        assert_exact_receipt(&db, &configure_request, &configured).await?;
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=false")
            .execute(&db.owner)
            .await?;

        let installation = Uuid::new_v4();
        let generation = Uuid::new_v4();
        let begin_key = format!("{generation}:begin");
        let make_intent = |artifact: Value, generation: Uuid, request_key: &str| {
            json!({
                "schema":"owner-beta-paper-operation-intent-v1",
                "installation_id":installation,
                "generation":generation,
                "account_id":account,
                "predecessor_generation":null,
                "request_key":request_key,
                "purpose":"STOP",
                "scope":"PAPER_STOP",
                "expected_epoch":0,
                "local_epoch_floor":0,
                "release_manifest_sha256":hash('a'),
                "artifact_sha256":artifact,
                "resume_target_id":null
            })
        };
        let intent = make_intent(Value::Null, generation, &begin_key);
        let mut begin_tx = caller.begin().await?;
        let actor_before: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *begin_tx)
                .await?;
        assert!(actor_before.as_deref().is_none_or(str::is_empty));
        let begun: Value =
            sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)")
                .bind(0_i64)
                .bind(&intent)
                .bind(&begin_key)
                .fetch_one(&mut *begin_tx)
                .await?;
        assert_eq!(begun["outcome"], "OK");
        assert_eq!(begun["control_epoch"], 1);
        assert_actor(&mut begin_tx, &actor_before).await?;
        begin_tx.commit().await?;
        let begin_request = exact_request("BEGIN",Some(account),json!({"expected_epoch":0,"intent_canonical":exact_json(&intent),"request_key":begin_key}));
        assert_exact_receipt(&db, &begin_request, &begun).await?;

        let foreign = Uuid::new_v4().to_string();
        let actor_before = Some(foreign.clone());
        let mut replay_tx = caller.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(&foreign)
            .execute(&mut *replay_tx)
            .await?;
        let replay: Value =
            sqlx::query_scalar("SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)")
                .bind(0_i64)
                .bind(&intent)
                .bind(&begin_key)
                .fetch_one(&mut *replay_tx)
                .await?;
        assert_eq!(replay["outcome"], "REPLAY");
        assert_actor(&mut replay_tx, &actor_before).await?;

        sqlx::query("SAVEPOINT changed_begin")
            .execute(&mut *replay_tx)
            .await?;
        let changed = make_intent(json!(hash('c')), generation, &begin_key);
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
            )
            .bind(0_i64)
            .bind(changed)
            .bind(&begin_key)
            .fetch_one(&mut *replay_tx)
            .await,
            "23505",
        );
        sqlx::query("ROLLBACK TO SAVEPOINT changed_begin")
            .execute(&mut *replay_tx)
            .await?;
        assert_actor(&mut replay_tx, &actor_before).await?;

        sqlx::query("SAVEPOINT stale_begin")
            .execute(&mut *replay_tx)
            .await?;
        let stale_generation = Uuid::new_v4();
        let stale_key = format!("{stale_generation}:begin");
        let stale_intent = make_intent(Value::Null, stale_generation, &stale_key);
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
            )
            .bind(0_i64)
            .bind(stale_intent)
            .bind(&stale_key)
            .fetch_one(&mut *replay_tx)
            .await,
            "40001",
        );
        sqlx::query("ROLLBACK TO SAVEPOINT stale_begin")
            .execute(&mut *replay_tx)
            .await?;
        assert_actor(&mut replay_tx, &actor_before).await?;
        replay_tx.commit().await?;

        let mut empty_actor_tx = caller.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id','',true)")
            .execute(&mut *empty_actor_tx)
            .await?;
        let empty_actor = Some(String::new());
        let empty_actor_replay: Value = sqlx::query_scalar(
            "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
        )
        .bind(0_i64)
        .bind(&intent)
        .bind(&begin_key)
        .fetch_one(&mut *empty_actor_tx)
        .await?;
        assert_eq!(empty_actor_replay["outcome"], "REPLAY");
        assert_actor(&mut empty_actor_tx, &empty_actor).await?;
        empty_actor_tx.rollback().await?;

        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
            )
            .bind(0_i64)
            .bind(&intent)
            .bind(&begin_key)
            .fetch_one(&db.app)
            .await,
            "42501",
        );
        let mut app_tx = db.app.begin().await?;
        denied(
            sqlx::query("SET LOCAL ROLE migration_owner")
                .execute(&mut *app_tx)
                .await,
            "42501",
        );
        app_tx.rollback().await?;

        let mut set_role_tx = caller.begin().await?;
        sqlx::query("SET ROLE migration_owner")
            .execute(&mut *set_role_tx)
            .await?;
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
            )
            .bind(0_i64)
            .bind(&intent)
            .bind(&begin_key)
            .fetch_one(&mut *set_role_tx)
            .await,
            "42501",
        );
        set_role_tx.rollback().await?;

        let mut inspect_tx = caller.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *inspect_tx)
            .await?;
        let inspected: Value =
            sqlx::query_scalar("SELECT inspect_owner_beta_paper_operation_v1($1,NULL,1)")
                .bind(generation)
                .fetch_one(&mut *inspect_tx)
                .await?;
        assert_eq!(inspected["outcome"], "OK");
        assert_eq!(inspected["data"]["state"], "DRAINING");
        assert_eq!(inspected["data"]["control_epoch"], 1);
        assert_eq!(inspected["data"]["history_present"], true);
        assert_eq!(
            inspected["data"]["operation_event_id"],
            begun["data"]["operation_event_id"]
        );
        assert_eq!(inspected["data"]["reconciliation"], Value::Null);
        assert!(inspected["data"]["targets"].as_array().is_some_and(Vec::is_empty));
        assert_actor(&mut inspect_tx, &None).await?;
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT inspect_owner_beta_paper_operation_v1($1,NULL,$2)",
            )
            .bind(generation)
            .bind(Option::<i32>::None)
            .fetch_one(&mut *inspect_tx)
            .await,
            "22023",
        );
        inspect_tx.rollback().await?;

        let mut live_app_tx = db.app.begin().await?;
        let _: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *live_app_tx)
            .await?;
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)",
            )
            .bind(generation)
            .bind(1_i64)
            .bind(hash('b'))
            .bind(format!("{generation}:reconcile:1"))
            .fetch_one(&caller)
            .await,
            "55000",
        );
        live_app_tx.rollback().await?;

        let dead_job = Uuid::new_v4();
        let mut dead_job_seed = db.owner.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(db.owner_id.to_string())
            .execute(&mut *dead_job_seed)
            .await?;
        let now: String = sqlx::query_scalar("SELECT clock_timestamp()::text")
            .fetch_one(&mut *dead_job_seed)
            .await?;
        sqlx::query("INSERT INTO jobs(id,owner_user_id,job_type,status,payload_json,max_attempts,attempt_count,locked_by,locked_at,started_at) VALUES($1,$2,'owner_beta_paper_preview','RUNNING','{}',3,1,'dead-paper-worker',$3::text::timestamptz,$3::text::timestamptz)")
            .bind(dead_job)
            .bind(db.owner_id)
            .bind(&now)
            .execute(&mut *dead_job_seed)
            .await?;
        sqlx::query("INSERT INTO job_attempts(job_id,attempt_no,outcome,claimed_by,started_at) VALUES($1,1,'RUNNING','dead-paper-worker',$2::text::timestamptz)")
            .bind(dead_job)
            .bind(&now)
            .execute(&mut *dead_job_seed)
            .await?;
        dead_job_seed.commit().await?;

        close_fixture_writer_connections(&db).await?;
        let inventory = hash('b');
        let reconcile_key = format!("{generation}:reconcile:1");
        let reconciled: Value = sqlx::query_scalar(
            "SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)",
        )
        .bind(generation)
        .bind(1_i64)
        .bind(&inventory)
        .bind(&reconcile_key)
        .fetch_one(&caller)
        .await?;
        assert_eq!(reconciled["outcome"], "OK");
        let reconcile_request = exact_request("RECONCILE",Some(account),json!({"generation":generation,"expected_epoch":1,"inventory_sha256":inventory,"request_key":reconcile_key}));
        assert_exact_receipt(&db, &reconcile_request, &reconciled).await?;
        assert_legacy_unproven(&db,&configure_request).await?;
        assert_legacy_unproven(&db,&begin_request).await?;
        assert_legacy_unproven(&db,&reconcile_request).await?;
        assert_eq!(
            reconciled["data"]["summary"]["database_sha256"]
                .as_str()
                .unwrap()
                .len(),
            71
        );
        assert_eq!(reconciled["data"]["summary"]["unresolved_commits"], 0);
        assert_eq!(reconciled["data"]["summary"]["live_claims"], 0);
        let dead_job_result: (String, String, String) = sqlx::query_as(
            "SELECT j.status,a.outcome,a.error_code FROM jobs j JOIN job_attempts a ON a.job_id=j.id AND a.attempt_no=j.attempt_count WHERE j.id=$1",
        )
        .bind(dead_job)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(
            dead_job_result,
            (
                "FAILED".to_string(),
                "ORPHANED".to_string(),
                "owner_beta_paper_fenced".to_string()
            )
        );
        let observation_id = reconciled["data"]["observation_id"].as_str().unwrap();
        let summary_sha256 = reconciled["data"]["summary_sha256"].as_str().unwrap();
        let reconcile_replay: Value = sqlx::query_scalar(
            "SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)",
        )
        .bind(generation)
        .bind(1_i64)
        .bind(&inventory)
        .bind(&reconcile_key)
        .fetch_one(&caller)
        .await?;
        assert_eq!(reconcile_replay["outcome"], "REPLAY");
        assert_eq!(
            reconcile_replay["data"]["observation_id"],
            observation_id
        );
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)",
            )
            .bind(generation)
            .bind(1_i64)
            .bind(hash('c'))
            .bind(&reconcile_key)
            .fetch_one(&caller)
            .await,
            "23505",
        );

        let observed_ns: i64 = sqlx::query_scalar(
            "SELECT floor(extract(epoch FROM clock_timestamp())*1000000000)::bigint",
        )
        .fetch_one(&db.owner)
        .await?;
        let off_key = format!("{generation}:off");
        let make_receipt = |installation_id: Uuid| {
            json!({
                "schema":"owner-beta-paper-drain-receipt-v1",
                "installation_id":installation_id,
                "generation":generation,
                "account_id":account,
                "request_key":off_key,
                "scope":"PAPER_STOP",
                "expected_epoch":0,
                "resulting_epoch":1,
                "intent_sha256":begun["data"]["intent_sha256"],
                "stage_sha256":hash('c'),
                "inventory_sha256":inventory,
                "reconciliation_sha256":summary_sha256,
                "observation_id":observation_id,
                "release_manifest_sha256":hash('a'),
                "boot_id":Uuid::new_v4(),
                "observed":{"realtime_ns":observed_ns,"monotonic_ns":1,"boottime_ns":1}
            })
        };
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)",
            )
            .bind(1_i64)
            .bind(make_receipt(Uuid::new_v4()))
            .bind(&off_key)
            .fetch_one(&caller)
            .await,
            "23514",
        );
        let mut malformed = make_receipt(installation);
        malformed.as_object_mut().unwrap().remove("boot_id");
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)",
            )
            .bind(1_i64)
            .bind(malformed)
            .bind(&off_key)
            .fetch_one(&caller)
            .await,
            "22023",
        );
        let mut oversized_epoch = make_receipt(installation);
        oversized_epoch["expected_epoch"] = json!(9223372036854775808u64);
        // Outside signed i64 is malformed canonical wire (22023), before
        // semantic receipt checks (23514). Neither rejection may publish OFF.
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)",
            )
            .bind(1_i64)
            .bind(oversized_epoch)
            .bind(&off_key)
            .fetch_one(&caller)
            .await,
            "22023",
        );
        let unchanged_epoch: i64 =
            sqlx::query_scalar("SELECT control_epoch FROM owner_beta_paper_control")
                .fetch_one(&db.owner)
                .await?;
        assert_eq!(unchanged_epoch, 1);
        let off_events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_events WHERE request_key=$1")
                .bind(&off_key)
                .fetch_one(&db.owner)
                .await?;
        assert_eq!(off_events, 0);
        let receipt = make_receipt(installation);
        let off: Value =
            sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)")
                .bind(1_i64)
                .bind(&receipt)
                .bind(&off_key)
                .fetch_one(&caller)
                .await?;
        assert_eq!(off["outcome"], "OK");
        assert_eq!(off["control_epoch"], 2);
        assert_exact_transition(&db,account,1,"OFF",&receipt,&off_key,&off).await?;
        assert_exact_receipt(&db,&reconcile_request,&reconciled).await?;
        assert_exact_receipt(&db,&begin_request,&begun).await?;
        assert_exact_receipt(&db,&configure_request,&configured).await?;
        let reset = exact_read(&caller,&exact_request("RESET",Some(account),json!({"expected_epoch":2}))).await?;
        assert_eq!(reset["reset"]["event_id"],off["data"]["event_id"]);
        assert_eq!(reset["reset"]["kind"],"OFF");
        let off_replay: Value =
            sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)")
                .bind(1_i64)
                .bind(&receipt)
                .bind(&off_key)
                .fetch_one(&caller)
                .await?;
        assert_eq!(off_replay["outcome"], "REPLAY");
        let mut changed_receipt = receipt.clone();
        changed_receipt["stage_sha256"] = json!(hash('d'));
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,'OFF',$2,$3)",
            )
            .bind(1_i64)
            .bind(changed_receipt)
            .bind(&off_key)
            .fetch_one(&caller)
            .await,
            "23505",
        );

        let mut final_inspect = caller.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *final_inspect)
            .await?;
        let readback: Value =
            sqlx::query_scalar("SELECT inspect_owner_beta_paper_operation_v1($1,NULL,1)")
                .bind(generation)
                .fetch_one(&mut *final_inspect)
                .await?;
        assert_eq!(readback["data"]["state"], "OFF");
        assert_eq!(readback["data"]["control_epoch"], 2);
        assert_eq!(
            readback["data"]["reconciliation"]["observation_id"],
            observation_id
        );
        final_inspect.rollback().await?;

        let restored_installation = Uuid::new_v4();
        let restore_generation = Uuid::new_v4();
        let restore_begin_key = format!("{restore_generation}:begin");
        let restore_intent = json!({
            "schema":"owner-beta-paper-operation-intent-v1",
            "installation_id":restored_installation,
            "generation":restore_generation,
            "account_id":account,
            "predecessor_generation":generation,
            "request_key":restore_begin_key,
            "purpose":"RESTORE",
            "scope":"COMBINED_BACKUP",
            "expected_epoch":2,
            "local_epoch_floor":2,
            "release_manifest_sha256":hash('a'),
            "artifact_sha256":hash('f'),
            "resume_target_id":null
        });
        let restore_begin: Value = sqlx::query_scalar(
            "SELECT begin_owner_beta_paper_operation_v1($1,$2,$3)",
        )
        .bind(2_i64)
        .bind(&restore_intent)
        .bind(&restore_begin_key)
        .fetch_one(&caller)
        .await?;
        assert_eq!(restore_begin["control_epoch"], 3);
        let restore_inventory = hash('c');
        let restore_reconcile_key = format!("{restore_generation}:reconcile:1");
        let restore_reconciled: Value = sqlx::query_scalar(
            "SELECT reconcile_owner_beta_paper_operation_v1($1,$2,$3,$4)",
        )
        .bind(restore_generation)
        .bind(3_i64)
        .bind(&restore_inventory)
        .bind(&restore_reconcile_key)
        .fetch_one(&caller)
        .await?;
        let restore_observed_ns: i64 = sqlx::query_scalar(
            "SELECT floor(extract(epoch FROM clock_timestamp())*1000000000)::bigint",
        )
        .fetch_one(&db.owner)
        .await?;
        let restore_key = format!("{restore_generation}:restore-off");
        let restore_receipt = json!({
            "schema":"owner-beta-paper-restore-receipt-v1",
            "installation_id":restored_installation,
            "generation":restore_generation,
            "account_id":account,
            "request_key":restore_key,
            "scope":"COMBINED_BACKUP",
            "expected_epoch":2,
            "resulting_epoch":3,
            "intent_sha256":restore_begin["data"]["intent_sha256"],
            "stage_sha256":hash('d'),
            "inventory_sha256":restore_inventory,
            "reconciliation_sha256":restore_reconciled["data"]["summary_sha256"],
            "observation_id":restore_reconciled["data"]["observation_id"],
            "release_manifest_sha256":hash('a'),
            "boot_id":Uuid::new_v4(),
            "observed":{"realtime_ns":restore_observed_ns,"monotonic_ns":1,"boottime_ns":1},
            "source_installation_id":installation,
            "source_generation":generation,
            "source_epoch":1,
            "archive_sha256":hash('f'),
            "source_audit_sha256":hash('7'),
            "restored_audit_sha256":hash('7'),
            "destination_epoch_floor":2
        });
        let mut wrong_destination_receipt = restore_receipt.clone();
        wrong_destination_receipt["installation_id"] = json!(installation);
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,'RESTORED_OFF',$2,$3)",
            )
            .bind(3_i64)
            .bind(wrong_destination_receipt)
            .bind(&restore_key)
            .fetch_one(&caller)
            .await,
            "23514",
        );
        let restored: Value = sqlx::query_scalar(
            "SELECT transition_owner_beta_paper_v1($1,'RESTORED_OFF',$2,$3)",
        )
        .bind(3_i64)
        .bind(&restore_receipt)
        .bind(&restore_key)
        .fetch_one(&caller)
        .await?;
        assert_eq!(restored["outcome"], "OK");
        assert_eq!(restored["control_epoch"], 4);
        assert_exact_transition(&db,account,3,"RESTORED_OFF",&restore_receipt,&restore_key,&restored).await?;
        let reset = exact_read(&caller,&exact_request("RESET",Some(account),json!({"expected_epoch":4}))).await?;
        assert_eq!(reset["reset"]["event_id"],restored["data"]["event_id"]);
        assert_eq!(reset["reset"]["kind"],"RESTORED_OFF");
        assert_exact_receipt(&db,&reconcile_request,&reconciled).await?;
        let mut restored_inspect = caller.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *restored_inspect)
            .await?;
        let restored_readback: Value = sqlx::query_scalar(
            "SELECT inspect_owner_beta_paper_operation_v1($1,NULL,1)",
        )
        .bind(restore_generation)
        .fetch_one(&mut *restored_inspect)
        .await?;
        assert_eq!(restored_readback["data"]["state"], "RESTORED_OFF");
        assert_eq!(restored_readback["data"]["control_epoch"], 4);
        restored_inspect.rollback().await?;

        let mut down_tx = caller.begin().await?;
        sqlx::query("SAVEPOINT nonempty_down")
            .execute(&mut *down_tx)
            .await?;
        denied(
            sqlx::raw_sql(PAPER_0068_DOWN)
                .execute(&mut *down_tx)
                .await,
            "23514",
        );
        sqlx::query("ROLLBACK TO SAVEPOINT nonempty_down")
            .execute(&mut *down_tx)
            .await?;
        down_tx.rollback().await?;
        Ok(())
    }
    .await;
    caller.close().await;
    db.close().await?;
    result
}

async fn isolated_db() -> Daily0060Result<Daily0060Db> {
    let url = require_r5_03_database_url()?;
    let admin = admin_pool(&url)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    verify_r5_03_postgres_identity(&admin).await?;
    admin.close().await;
    let mut db = Daily0060Db::new(url, 67).await?;
    // The real worker pool supplies no authenticated app actor. Do not let
    // the predecessor fixture's Owner GUC conceal a SECURITY DEFINER/RLS bug.
    db.worker.close().await;
    db.worker = worker_without_actor(&db, 3).await?;
    Ok(db)
}

async fn worker_without_actor(db: &Daily0060Db, connections: u32) -> Daily0060Result<PgPool> {
    login_without_actor(db, "worker", connections).await
}

async fn login_without_actor(
    db: &Daily0060Db,
    principal: &str,
    connections: u32,
) -> Daily0060Result<PgPool> {
    let options: sqlx::postgres::PgConnectOptions = db.url.parse()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(connections)
        .connect_with(
            options
                .username(principal)
                .database(&db.name)
                .options([("statement_timeout", "15000"), ("lock_timeout", "7000")]),
        )
        .await?;
    let identity: (String, String, Option<String>) = sqlx::query_as(
        "SELECT current_user,session_user,current_setting('app.actor_user_id',true)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        (identity.0.as_str(), identity.1.as_str()),
        (principal, principal)
    );
    assert!(
        identity.2.as_deref().is_none_or(str::is_empty),
        "fresh {principal} login must not inherit the configured Owner actor"
    );
    Ok(pool)
}

async fn assert_actor(
    connection: &mut sqlx::PgConnection,
    prior: &Option<String>,
) -> Daily0060Result {
    let after: Option<String> =
        sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
            .fetch_one(connection)
            .await?;
    if prior.is_none() {
        // PostgreSQL may retain a custom-GUC placeholder once referenced; empty
        // is the same unbound actor, and must never become the configured Owner.
        assert!(
            after.as_deref().is_none_or(str::is_empty),
            "unset worker actor leaked Owner scope: {after:?}"
        );
    } else {
        assert_eq!(
            &after, prior,
            "function must restore the exact caller actor before returning"
        );
    }
    Ok(())
}

async fn actor_error(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    result: Result<Value, sqlx::Error>,
    code: &str,
    prior: &Option<String>,
) -> Daily0060Result {
    denied(result, code);
    sqlx::query("ROLLBACK TO SAVEPOINT actor_error")
        .execute(&mut **tx)
        .await?;
    sqlx::query("RELEASE SAVEPOINT actor_error")
        .execute(&mut **tx)
        .await?;
    assert_actor(tx, prior).await
}

async fn transition_with_actor_checks(
    db: &Daily0060Db,
    epoch: i64,
    transition: &str,
    evidence: &Value,
    request: &str,
) -> Daily0060Result<Value> {
    let foreign = Uuid::new_v4().to_string();
    for actor in [None, Some(""), Some(foreign.as_str())] {
        let pool = login_without_actor(db, "migration_owner", 1).await?;
        let case: Daily0060Result = async {
            let mut tx = pool.begin().await?;
            if let Some(actor) = actor {
                sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
                    .bind(actor)
                    .execute(&mut *tx)
                    .await?;
            }
            let prior: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *tx)
                    .await?;
            let applied: Value =
                sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
                    .bind(epoch)
                    .bind(transition)
                    .bind(evidence)
                    .bind(request)
                    .fetch_one(&mut *tx)
                    .await?;
            assert_eq!(applied["outcome"], "OK");
            assert_actor(&mut tx, &prior).await?;
            let replay: Value =
                sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
                    .bind(epoch)
                    .bind(transition)
                    .bind(evidence)
                    .bind(request)
                    .fetch_one(&mut *tx)
                    .await?;
            assert_eq!(replay["outcome"], "REPLAY");
            assert_actor(&mut tx, &prior).await?;
            let mut changed = evidence.clone();
            if transition == "ENABLE" {
                changed["new_epoch"] = json!(3);
            } else {
                changed["decision_sha256"] = json!(hash('9'));
            }
            sqlx::query("SAVEPOINT actor_error")
                .execute(&mut *tx)
                .await?;
            let error = sqlx::query_scalar::<_, Value>(
                "SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)",
            )
            .bind(epoch)
            .bind(transition)
            .bind(&changed)
            .bind(request)
            .fetch_one(&mut *tx)
            .await;
            actor_error(&mut tx, error, "23505", &prior).await?;
            tx.rollback().await?;
            Ok(())
        }
        .await;
        pool.close().await;
        case?;
    }
    let pool = login_without_actor(db, "migration_owner", 1).await?;
    let committed: Daily0060Result<Value> = async {
        let mut tx = pool.begin().await?;
        let prior: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        let applied: Value =
            sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,$2,$3,$4)")
                .bind(epoch)
                .bind(transition)
                .bind(evidence)
                .bind(request)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(applied["outcome"], "OK");
        assert_actor(&mut tx, &prior).await?;
        tx.commit().await?;
        Ok(applied)
    }
    .await;
    pool.close().await;
    committed
}

fn hash(digit: char) -> String {
    super::daily_0060_hash_value(digit)
}

fn financial_contract() -> Value {
    json!({"contract_id":"owner-beta-paper-v1-draft-1",
        "contract_sha256":"sha256:18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0",
        "decision_sha256":"sha256:8ef602a35586d3e2ca532c5fbb5efc6f2e727eb25ebeb45a44dad0ea0d0ca394",
        "cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1})
}

async fn configure(db: &Daily0060Db) -> Daily0060Result<(Uuid, Uuid, Value)> {
    db.control(true).await?;
    sqlx::query("INSERT INTO strategies(id,display_name) VALUES('trend_following','Synthetic SQL strategy') ON CONFLICT DO NOTHING").execute(&db.owner).await?;
    let config: Uuid=sqlx::query_scalar("INSERT INTO user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json,daily_enabled) VALUES($1,'trend_following','1.0.0','{\"benchmark_instrument\":\"069500.KRX\",\"fast_ma\":50,\"slow_ma\":200}',true) RETURNING id").bind(db.owner_id).fetch_one(&db.app).await?;
    let result: Value = sqlx::query_scalar(
        "SELECT configure_owner_beta_paper_v1($1,'SQL fixture',$2,NULL::jsonb,'sql-config')",
    )
    .bind(config)
    .bind(financial_contract())
    .fetch_one(&db.owner)
    .await?;
    assert_eq!(result["outcome"], "OK");
    let account = Uuid::parse_str(result["account_id"].as_str().ok_or("account id")?)?;
    Ok((account, config, result))
}

// Closing synthetic client pools proves this fixture's backend absence only;
// it does not replace the production controller's process/claim/fence proof.
async fn close_fixture_writer_connections(db: &Daily0060Db) -> Daily0060Result {
    db.app.close().await;
    db.worker.close().await;
    for _ in 0..100 {
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND usename IN ('app','worker')",
        )
        .fetch_one(&db.owner)
        .await?;
        if remaining == 0 {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err("synthetic writer backends did not disappear".into())
}

#[track_caller]
fn denied<T>(result: Result<T, sqlx::Error>, code: &str) {
    let error = result.err().expect("operation must be denied");
    let sqlstate = error
        .as_database_error()
        .and_then(|e| e.code())
        .filter(|value| {
            value.len() == 5
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        });
    assert_eq!(
        sqlstate.as_deref(),
        Some(code),
        "SQLSTATE={}",
        sqlstate.as_deref().unwrap_or("NON_DATABASE")
    );
}

async fn wait_for_pg_blocker(
    db: &Daily0060Db,
    waiter_pid: i32,
    blocker_pid: i32,
) -> Daily0060Result {
    for _ in 0..120 {
        let blockers: Vec<i32> = sqlx::query_scalar("SELECT pg_blocking_pids($1)")
            .bind(waiter_pid)
            .fetch_one(&db.owner)
            .await?;
        if blockers.contains(&blocker_pid) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    Err(
        format!("backend {waiter_pid} did not show blocker {blocker_pid} in pg_blocking_pids")
            .into(),
    )
}

async fn backend_waits_on_advisory_lock(db: &Daily0060Db, pid: i32) -> Daily0060Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)",
    )
    .bind(pid)
    .fetch_one(&db.owner)
    .await?)
}

fn assert_same_actor(before: &Option<String>, after: &Option<String>) {
    assert_eq!(
        before.as_deref().unwrap_or(""),
        after.as_deref().unwrap_or(""),
        "function or trigger must preserve the caller actor"
    );
}

async fn calendar_claim(
    connection: &mut sqlx::PgConnection,
    owner: Uuid,
    evidence: &str,
    policy: &str,
    days: &Value,
    future: bool,
) -> Daily0060Result {
    // SYNTHETIC calendar facts exercise SQL selection only, never Raw/source sealing.
    sqlx::query("INSERT INTO owner_beta_daily_calendar_sources(calendar_evidence_sha256,owner_user_id,policy_sha256,normalizer_version,source_provider,source_market,source_batch_id,source_batch_date,requested_target,source_file_name,source_file_sha256,source_manifest_line_sha256,source_retrieved_at,copy_chain,evidence_json,projected_at) VALUES($1,$2,$3,'daily-raw-reconstruction-v1','kis','kr',gen_random_uuid(),current_date,current_date,'synthetic-calendar.json',$1,$1,clock_timestamp()+CASE WHEN $4 THEN interval '1 day' ELSE interval '0' END,'[]','{}',clock_timestamp())")
        .bind(evidence).bind(owner).bind(policy).bind(future).execute(&mut *connection).await?;
    sqlx::query("INSERT INTO owner_beta_daily_calendar_days(owner_user_id,calendar_evidence_sha256,session_date,session_type,evidence_kind,source_row_ordinal,source_row_sha256) SELECT $1,$2,(x->>'date')::date,x->>'kind','DERIVED_SURROUNDING',(x->>'ordinal')::int,x->>'row_hash' FROM jsonb_array_elements($3) x")
        .bind(owner).bind(evidence).bind(days).execute(&mut *connection).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0067_calendar_agreement_conflict_coverage_and_observation() -> Daily0060Result {
    let db = isolated_db().await?;
    let result:Daily0060Result=async {
        let (today,one,two):(String,String,String)=sqlx::query_as("SELECT current_date::text,(current_date+1)::text,(current_date+2)::text").fetch_one(&db.owner).await?;
        let days=|a,b|json!([{"date":one,"kind":"CLOSED","ordinal":a,"row_hash":hash('d')},{"date":two,"kind":"TRADING","ordinal":b,"row_hash":hash('e')}]);
        let mut c=db.owner.acquire().await?;
        calendar_claim(&mut c,db.owner_id,&hash('b'),&hash('1'),&days(2,3),false).await?;
        calendar_claim(&mut c,db.owner_id,&hash('a'),&hash('1'),&days(90,91),false).await?;
        // Wrong-policy contradictory facts cannot fill coverage or create a conflict.
        calendar_claim(&mut c,db.owner_id,&hash('0'),&hash('2'),&json!([{"date":one,"kind":"TRADING","ordinal":0,"row_hash":hash('f')}]),false).await?;
        let read=|after:String|sqlx::query_scalar::<_,Value>("SELECT calendar_link_json FROM paper_0067_next_session($1,$2,$3::text::date)").bind(db.owner_id).bind(hash('1')).bind(after);
        let link=read(today.clone()).fetch_one(&mut *c).await?;
        assert_eq!(link["effective_session"],two);
        assert_eq!(link["covered_days"][0]["calendar_evidence_sha256"],hash('a'));
        assert_eq!(link["covered_days"][0]["row_ordinal"],90);
        assert_eq!(link["covered_days"][0]["row_sha256"],hash('d'));
        for future in [false,true] {
            let mut tx=c.begin().await?;
            calendar_claim(&mut tx,db.owner_id,&hash('c'),&hash('1'),&json!([{"date":one,"kind":if future {"CLOSED"} else {"TRADING"},"ordinal":1,"row_hash":hash('f')}]),future).await?;
            assert!(read(today.clone()).fetch_optional(&mut *tx).await?.is_none(),"conflict or unobserved source must deny");
            tx.rollback().await?;
        }
        calendar_claim(&mut c,db.owner_id,&hash('f'),&hash('3'),&json!([{"date":two,"kind":"TRADING","ordinal":0,"row_hash":hash('f')}]),false).await?;
        let gap:i64=sqlx::query_scalar("SELECT count(*) FROM paper_0067_next_session($1,$2,$3::text::date)").bind(db.owner_id).bind(hash('3')).bind(today).fetch_one(&mut *c).await?;
        assert_eq!(gap,0,"missing civil day must deny");
        Ok(())
    }.await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0067_configuration_consent_roles_rls_and_history_down() -> Daily0060Result {
    let db = isolated_db().await?;
    let result:Daily0060Result=async {
        let (account,config,first)=configure(&db).await?;
        let replay:Value=sqlx::query_scalar("SELECT configure_owner_beta_paper_v1($1,'SQL fixture',$2,NULL::jsonb,'sql-config')").bind(config).bind(financial_contract()).fetch_one(&db.owner).await?;
        assert_eq!(replay["outcome"],"REPLAY"); assert_eq!(replay["account_id"],first["account_id"]);
        let deposit:(i64,String)=sqlx::query_as("SELECT count(*),sum(amount)::text FROM cash_ledger WHERE account_id=$1").bind(account).fetch_one(&db.owner).await?;
        assert_eq!(deposit,(1,"10000000.0000".into()));
        let consent=||sqlx::query_scalar::<_,Value>("SELECT record_owner_beta_paper_consent_v1($1,0,'owner-beta-paper-v1-draft-1',$2,true,'grant')").bind(account).bind(financial_contract()["contract_sha256"].as_str().unwrap().to_owned());
        let granted=consent().fetch_one(&db.app).await?; assert_eq!(granted["outcome"],"OK");
        let replay=consent().fetch_one(&db.app).await?; assert_eq!(replay["outcome"],"REPLAY"); assert_eq!(replay["data"]["event_id"],granted["data"]["event_id"]);
        denied(consent().fetch_one(&db.worker).await,"42501");
        denied(sqlx::query("SELECT configure_owner_beta_paper_v1($1,'x',$2,NULL::jsonb,'forged')").bind(config).bind(financial_contract()).execute(&db.app).await,"42501");
        denied(sqlx::query("UPDATE accounts SET name='forged' WHERE id=$1").bind(account).execute(&db.app).await,"42501");
        denied(sqlx::query("SELECT lock_paper_rebalance_preview_submission($1,$2,$3,current_date)").bind(db.owner_id).bind(account).bind(Uuid::new_v4()).execute(&db.app).await,"42501");
        let foreign=Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'https://sql.invalid','foreign','foreign@invalid')").bind(foreign).execute(&db.owner).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id) VALUES($1,'owner')").bind(foreign).execute(&db.owner).await?;
        let other=super::daily_0060_login(&db.url,&db.name,"app",foreign).await?;
        let hidden:i64=sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_control").fetch_one(&other).await?; assert_eq!(hidden,0);
        assert_eq!(consent().fetch_one(&other).await?["outcome"],"NOT_FOUND");
        other.close().await;
        let admin=admin_pool(&super::supervisor_db_url(&db.url,&db.name)).await.map_err(|e|std::io::Error::other(e.to_string()))?;
        let mut forged=admin.begin().await?;
        sqlx::query("SET LOCAL ROLE app").execute(&mut *forged).await?;
        denied(sqlx::query("SELECT read_owner_beta_paper_state_v1()").execute(&mut *forged).await,"42501"); forged.rollback().await?;admin.close().await;
        let before=paper_catalog(&mut *db.owner.acquire().await?).await?;
        let mut tx=db.owner.begin().await?;
        denied(sqlx::raw_sql(PAPER_0067_DOWN).execute(&mut *tx).await,"23514");tx.rollback().await?;
        assert_eq!(paper_catalog(&mut *db.owner.acquire().await?).await?,before,"history refusal must precede any DDL");
        sqlx::query("DELETE FROM user_roles WHERE user_id=$1 AND role_id='owner'").bind(db.owner_id).execute(&db.owner).await?;
        denied(consent().fetch_one(&db.app).await,"42501");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM owner_beta_paper_control").fetch_one(&db.app).await?,0,"revoked Owner membership must hide direct reads");
        sqlx::query("INSERT INTO user_roles(user_id,role_id) VALUES($1,'owner')").bind(db.owner_id).execute(&db.owner).await?;
        let revoked:Value=sqlx::query_scalar("SELECT record_owner_beta_paper_consent_v1($1,0,'owner-beta-paper-v1-draft-1',$2,false,'revoke')").bind(account).bind(financial_contract()["contract_sha256"].as_str().unwrap().to_owned()).fetch_one(&db.app).await?;
        assert_eq!(revoked["control_epoch"],1);
        denied(sqlx::query("SELECT record_owner_beta_paper_consent_v1($1,0,'owner-beta-paper-v1-draft-1',$2,true,'stale')").bind(account).bind(financial_contract()["contract_sha256"].as_str().unwrap().to_owned()).execute(&db.app).await,"40001");
        let preserved:i64=sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_events").fetch_one(&db.owner).await?;assert_eq!(preserved,3);
        Ok(())
    }.await;
    db.close().await?;
    result
}

async fn target_snapshot(db: &Daily0060Db, f: &SqlPaper, target: Uuid) -> Daily0060Result<Value> {
    Ok(
        sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,1,$2)")
            .bind(target)
            .bind(&f.digest)
            .fetch_one(&db.worker)
            .await?,
    )
}
async fn observation(db: &Daily0060Db) -> Daily0060Result<Value> {
    Ok(sqlx::query_scalar("SELECT jsonb_build_object('original_retrieved_at',clock_timestamp()-interval '10 seconds','admitted_at',clock_timestamp()-interval '20 seconds','usable_at',clock_timestamp(),'precision','APPLICATION_OBSERVED')").fetch_one(&db.owner).await?)
}
async fn settlement_packet(
    db: &Daily0060Db,
    f: &SqlPaper,
    target: Uuid,
    admission: &str,
    trade: bool,
) -> Daily0060Result<Value> {
    let s = target_snapshot(db, f, target).await?;
    let s = &s["data"];
    let positions = if trade {
        json!([{"instrument_id":"069500.KRX","quantity":"10"}])
    } else {
        Value::Array(s["account_state_preimage"]["positions"].as_object().ok_or("positions")?.iter().map(|(i,q)|json!({"instrument_id":i,"quantity":q.as_str().unwrap().split('.').next().unwrap()})).collect())
    };
    Ok(
        json!({"schema":"owner-beta-paper-settlement-v1","target_id":target.to_string(),"lineage_id":s["lineage_id"],"control_epoch":1,"execution_admission_sha256":admission,"execution_input_sha256":hash('a'),"account_state_version":s["account_state_version"],"account_state_sha256":s["initial_account_state_sha256"],"ledger_before_sha256":format!("sha256:{}",s["initial_account_state_sha256"].as_str().unwrap()),"outcome":if trade {"TRADE"} else {"NO_TRADE"},"reason_code":if trade {"REBALANCED"} else {"SIGNAL_UNCHANGED"},"observed_input":observation(db).await?,"orders":if trade {json!([{"order_ref":"00000000-0000-5000-8000-000000000001","fill_ref":"00000000-0000-5000-8000-000000000002","instrument_id":"069500.KRX","side":"BUY","quantity":"10","raw_open":"10000.0000","fill_price":"10010.0000","commission":"1000.0000","tax":"0.0000","slippage":"100.0000","cash_before":"10000000.0000","cash_after":"9898900.0000"}])} else {json!([])},"positions_after":positions,"cash_after":if trade {json!("9898900.0000")} else {s["account_state_preimage"]["cash"].clone()},"calculation_sha256":if trade {hash('b')} else {hash('c')}}),
    )
}
async fn settle(
    db: &Daily0060Db,
    f: &SqlPaper,
    target: Uuid,
    packet: &Value,
) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT commit_owner_beta_paper_settlement_v1($1,1,$2,$3)")
        .bind(target)
        .bind(packet)
        .bind(&f.digest)
        .fetch_one(&db.worker)
        .await
}
async fn valuation_packet(
    db: &Daily0060Db,
    f: &SqlPaper,
    target: Uuid,
    admission: &str,
    close: &str,
    equity: &str,
    value: &str,
) -> Daily0060Result<Value> {
    let s = target_snapshot(db, f, target).await?;
    let s = &s["data"];
    assert!(
        s["account_state_preimage"]["cash"].is_string(),
        "settlement preimage cash must remain a string"
    );
    let held = !s["account_state_preimage"]["positions"]
        .as_object()
        .ok_or("positions")?
        .is_empty();
    Ok(
        json!({"schema":"owner-beta-paper-valuation-v1","target_id":target.to_string(),"control_epoch":1,"valuation_admission_sha256":admission,"valuation_input_sha256":hash('d'),"settlement_sha256":s["settlement_sha256"],"ledger_after_sha256":s["ledger_after_sha256"],"account_state_version":s["account_state_version"],"account_state_sha256":s["account_state_sha256"],"closes":if held {json!([{"instrument_id":"069500.KRX","raw_close":close}])} else {json!([])},"equity":equity,"cash":s["account_state_preimage"]["cash"],"positions_value":value,"currency":"KRW","observed_input":observation(db).await?,"calculation_sha256":if close=="10100.0000" {hash('e')} else {hash('f')}}),
    )
}
async fn valuate(
    db: &Daily0060Db,
    f: &SqlPaper,
    target: Uuid,
    packet: &Value,
) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT commit_owner_beta_paper_valuation_v1($1,1,$2,$3)")
        .bind(target)
        .bind(packet)
        .bind(&f.digest)
        .fetch_one(&db.worker)
        .await
}
fn once_and_replay(a: &Value, b: &Value) {
    let mut outcomes = [
        a["outcome"].as_str().unwrap(),
        b["outcome"].as_str().unwrap(),
    ];
    outcomes.sort();
    assert_eq!(outcomes, ["OK", "REPLAY"]);
}
async fn auto_target(
    db: &Daily0060Db,
    f: &SqlPaper,
    trade: bool,
) -> Daily0060Result<(Uuid, Value)> {
    let (preview, job, packet) = preview_packet(db, f, "AUTO", trade).await?;
    let mut tx = db.worker.begin().await?;
    let result = publish_in(&mut tx, f, preview, job, &packet).await?;
    assert_eq!(result["outcome"], "OK", "{result}");
    finish_preview(&mut tx, job).await?;
    tx.commit().await?;
    Ok((
        Uuid::parse_str(
            result["data"]["pending_target_id"]
                .as_str()
                .ok_or("target")?,
        )?,
        packet,
    ))
}
async fn economic_counts(
    db: &Daily0060Db,
    account: Uuid,
) -> Daily0060Result<(i64, i64, i64, i64, i64)> {
    Ok(sqlx::query_as("SELECT (SELECT count(*) FROM orders WHERE account_id=$1),(SELECT count(*) FROM fills WHERE account_id=$1),(SELECT count(*) FROM cash_ledger WHERE account_id=$1),(SELECT count(*) FROM paper_settlement_outbox WHERE owner_user_id=$2),(SELECT count(*) FROM daily_equity WHERE account_id=$1)").bind(account).bind(db.owner_id).fetch_one(&db.owner).await?)
}

async fn economic_image(db: &Daily0060Db, account: Uuid) -> Daily0060Result<Value> {
    Ok(sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'account',(SELECT to_jsonb(a) FROM accounts a WHERE a.id=$1),
      'binding',(SELECT jsonb_agg(to_jsonb(b) ORDER BY b.id) FROM account_strategy_bindings b WHERE b.account_id=$1),
      'cash',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.seq) FROM cash_ledger c WHERE c.account_id=$1),
      'positions',(SELECT jsonb_agg(to_jsonb(p) ORDER BY p.instrument_id) FROM positions p WHERE p.account_id=$1),
      'orders',(SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM orders o WHERE o.account_id=$1),
      'fills',(SELECT jsonb_agg(to_jsonb(f) ORDER BY f.id) FROM fills f WHERE f.account_id=$1),
      'targets',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM pending_targets t WHERE t.account_id=$1),
      'sessions',(SELECT jsonb_agg(to_jsonb(s) ORDER BY s.pending_target_id) FROM owner_beta_paper_sessions s WHERE s.account_id=$1),
      'session_events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.pending_target_id,e.event_seq) FROM owner_beta_paper_session_events e JOIN owner_beta_paper_sessions s ON s.pending_target_id=e.pending_target_id WHERE s.account_id=$1),
      'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY o.id) FROM paper_settlement_outbox o WHERE o.owner_user_id=$2),
      'equity',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.trading_date) FROM daily_equity e WHERE e.account_id=$1),
      'control',(SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE c.account_id=$1),
      'control_events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM owner_beta_paper_events e WHERE e.account_id=$1))"#)
        .bind(account).bind(db.owner_id).fetch_one(&db.owner).await?)
}

async fn rewrite_synthetic_daily_allocation(
    db: &Daily0060Db,
    f: &SqlPaper,
    invested: bool,
    variant: &str,
) -> Daily0060Result {
    // The same trusted SYNTHETIC predecessor seam used by seed_witness.
    // Invalid projections deliberately bypass only predecessor USER triggers;
    // restore their exact state before either serving principal reads the rows.
    // FK/CHECK/FORCE RLS and all 0067 guards remain active throughout.
    let (instrument, weight, excluded, cash) = match variant {
        "canonical" | "null_excluded" if invested => {
            ("069500.KRX", Some("1.000000"), false, "0.000000")
        }
        "canonical" | "null_excluded" => ("069500.KRX", Some("0.000000"), true, "1.000000"),
        "null_selected" => (
            "069500.KRX",
            None,
            false,
            if invested { "0.000000" } else { "1.000000" },
        ),
        "excluded_nonzero" => ("069500.KRX", Some("1.000000"), true, "0.000000"),
        "nonbenchmark_nonzero" => ("102110.KRX", Some("1.000000"), false, "0.000000"),
        "fractional" => ("069500.KRX", Some("0.500000"), false, "0.500000"),
        "cash_mismatch" if invested => ("069500.KRX", Some("1.000000"), false, "1.000000"),
        "cash_mismatch" => ("069500.KRX", Some("0.000000"), true, "0.000000"),
        _ => return Err("unknown synthetic allocation variant".into()),
    };
    let tables = [
        "owner_beta_recommendation_runs",
        "owner_beta_recommendation_items",
    ];
    let trigger_sql = "SELECT jsonb_agg(jsonb_build_array(c.relname,t.tgname,t.tgenabled) ORDER BY c.relname,t.tgname) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE NOT t.tgisinternal AND c.relname=ANY($1)";
    let mut tx = db.owner.begin().await?;
    let before: Value = sqlx::query_scalar(trigger_sql)
        .bind(&tables[..])
        .fetch_one(&mut *tx)
        .await?;
    for table in tables {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE public.{table} DISABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await?;
    }
    let changed = sqlx::query("UPDATE owner_beta_recommendation_items i SET target_weight=CASE WHEN i.instrument_id=$3 THEN $4::text::numeric ELSE 0 END,excluded=CASE WHEN i.instrument_id=$3 THEN $5 ELSE true END,rank=CASE WHEN i.instrument_id=$3 AND NOT $5 THEN 1 ELSE NULL END,exclusion_reason=CASE WHEN i.instrument_id=$3 AND NOT $5 THEN NULL WHEN i.instrument_id='069500.KRX' THEN 'TREND_NEGATIVE_CASH' ELSE 'NOT_SELECTED_BY_STRATEGY' END,reason_codes=jsonb_build_array(CASE WHEN i.instrument_id=$3 AND NOT $5 THEN 'TREND_POSITIVE' WHEN i.instrument_id='069500.KRX' THEN 'TREND_NEGATIVE_CASH' ELSE 'NOT_SELECTED_BY_STRATEGY' END) WHERE i.recommendation_run_id=$1 AND i.owner_user_id=$2")
        .bind(f.witness.run).bind(db.owner_id).bind(instrument).bind(weight).bind(excluded).execute(&mut *tx).await?;
    assert_eq!(changed.rows_affected(), 11);
    if variant == "null_excluded" {
        let changed = sqlx::query("UPDATE owner_beta_recommendation_items SET target_weight=NULL WHERE recommendation_run_id=$1 AND owner_user_id=$2 AND instrument_id='102110.KRX' AND excluded")
            .bind(f.witness.run).bind(db.owner_id).execute(&mut *tx).await?;
        assert_eq!(changed.rows_affected(), 1);
    }
    let changed = sqlx::query("UPDATE owner_beta_recommendation_runs SET cash_weight=$3::text::numeric WHERE id=$1 AND owner_user_id=$2")
        .bind(f.witness.run).bind(db.owner_id).bind(cash).execute(&mut *tx).await?;
    assert_eq!(changed.rows_affected(), 1);
    for table in tables {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE public.{table} ENABLE TRIGGER USER"
        )))
        .execute(&mut *tx)
        .await?;
    }
    let after: Value = sqlx::query_scalar(trigger_sql)
        .bind(&tables[..])
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(
        before, after,
        "invalid synthetic allocation must restore every predecessor trigger before serving reads"
    );
    tx.commit().await?;
    Ok(())
}

fn allocation_denied(result: Result<Value, sqlx::Error>, mode: &str, variant: &str) {
    let error = result.expect_err("invalid Daily allocation must not queue a preview");
    let database = error
        .as_database_error()
        .expect("allocation refusal must be an actual SQL error");
    assert_eq!(
        database.code().as_deref(),
        Some("23514"),
        "{mode}/{variant}: {error}"
    );
    assert!(
        matches!(
            database.message(),
            "allocation outside adopted ETF11 signal"
                | "allocation weights violate fixed strategy"
                | "automatic allocation outside adopted ETF11 signal"
                | "automatic allocation violates fixed strategy"
        ),
        "{mode}/{variant} must reach the Paper allocation guard: {error}"
    );
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_daily_allocation_manual_auto_exclusions_cash_and_invalid_weights()
-> Daily0060Result {
    for invested in [true, false] {
        let db = isolated_db().await?;
        let clock = FixtureClock::new().await?;
        clock.after_cutoff().await?;
        let result: Daily0060Result = async {
            let f = enabled_paper(&db, invested).await?;
            let projection_sql = "SELECT jsonb_agg(jsonb_build_object('instrument_id',i.instrument_id,'weight',i.target_weight::text,'rank',i.rank,'excluded',i.excluded,'exclusion_reason',i.exclusion_reason,'reason_codes',i.reason_codes) ORDER BY i.instrument_id) FROM owner_beta_recommendation_items i WHERE i.recommendation_run_id=$1";
            let canonical: Value = sqlx::query_scalar(projection_sql).bind(f.witness.run).fetch_one(&db.owner).await?;
            let shape: (i64, i64, i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE excluded),count(*) FILTER(WHERE NOT excluded),count(*) FILTER(WHERE target_weight IS NULL OR (excluded AND (target_weight<>0 OR rank IS NOT NULL OR exclusion_reason IS NULL OR exclusion_reason='')) OR (NOT excluded AND (rank IS NULL OR exclusion_reason IS NOT NULL))) FROM owner_beta_recommendation_items WHERE recommendation_run_id=$1")
                .bind(f.witness.run).fetch_one(&db.owner).await?;
            assert_eq!(shape, (11, if invested { 10 } else { 11 }, if invested { 1 } else { 0 }, 0));
            let before = economic_image(&db, f.account).await?;
            let submissions_sql = "SELECT (SELECT count(*) FROM paper_rebalance_previews WHERE account_id=$1),(SELECT count(*) FROM owner_beta_paper_lineage WHERE account_id=$1),(SELECT count(*) FROM jobs WHERE owner_user_id=$2 AND job_type='owner_beta_paper_preview')";
            let initial: (i64, i64, i64) = sqlx::query_as(submissions_sql).bind(f.account).bind(db.owner_id).fetch_one(&db.owner).await?;
            assert_eq!(initial, (0, 0, 0));
            // Probe before any valid request exists: idempotent replay must not
            // conceal a malformed new allocation or bypass its serving guard.
            for variant in ["null_excluded", "null_selected", "excluded_nonzero", "nonbenchmark_nonzero", "fractional", "cash_mismatch"] {
                rewrite_synthetic_daily_allocation(&db, &f, invested, variant).await?;
                let manual = sqlx::query_scalar::<_, Value>("SELECT submit_owner_beta_paper_preview_v1($1,$2,1,$3,$4)")
                    .bind(f.account).bind(f.witness.run).bind(format!("invalid-{variant}")).bind(&f.digest).fetch_one(&db.app).await;
                allocation_denied(manual, "MANUAL", variant);
                let auto = sqlx::query_scalar::<_, Value>("SELECT queue_owner_beta_paper_target_v1($1,1,$2)")
                    .bind(f.witness.run).bind(&f.digest).fetch_one(&db.worker).await;
                allocation_denied(auto, "AUTO", variant);
                assert_eq!(before, economic_image(&db, f.account).await?, "{variant} must leave the ledger, control and outbox unchanged");
                let after: (i64, i64, i64) = sqlx::query_as(submissions_sql).bind(f.account).bind(db.owner_id).fetch_one(&db.owner).await?;
                assert_eq!(initial, after, "{variant} must not create a preview, lineage or job");
            }
            rewrite_synthetic_daily_allocation(&db, &f, invested, "canonical").await?;
            let restored: Value = sqlx::query_scalar(projection_sql).bind(f.witness.run).fetch_one(&db.owner).await?;
            assert_eq!(canonical, restored, "restore exact canonical Daily item projection after negative probes");
            let expected = if invested { json!([{"instrument_id":"069500.KRX","weight":"1.000000"}]) } else { json!([]) };
            for mode in ["MANUAL", "AUTO"] {
                let (preview, job, packet) = preview_packet(&db, &f, mode, invested).await?;
                let snapshot: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)")
                    .bind(preview).bind(job).bind(&f.digest).fetch_one(&db.worker).await?;
                assert_eq!(snapshot["outcome"], "OK");
                assert_eq!(snapshot["data"]["apply_mode"], mode);
                assert_eq!(snapshot["data"]["target_weights"], expected);
                let mut tx = db.worker.begin().await?;
                let published = publish_in(&mut tx, &f, preview, job, &packet).await?;
                assert_eq!(published["outcome"], "OK");
                finish_preview(&mut tx, job).await?;
                tx.commit().await?;
                let status: String = sqlx::query_scalar("SELECT status FROM paper_rebalance_previews WHERE id=$1").bind(preview).fetch_one(&db.owner).await?;
                assert_eq!(status, if mode == "MANUAL" { "READY" } else { "APPLIED" });
            }
            let applied: Value = sqlx::query_scalar("SELECT targets_json FROM pending_targets WHERE account_id=$1 AND source_kind='OWNER_BETA_DAILY_AUTO'").bind(f.account).fetch_one(&db.owner).await?;
            assert_eq!(applied, expected);
            assert_eq!(economic_counts(&db, f.account).await?, (0, 0, 1, 0, 0), "T-close allocation must not execute or value future E");
            Ok(())
        }.await;
        db.close().await?;
        clock.set(0).await?;
        result?;
    }
    Ok(())
}

async fn terminal_readback_without_mutation(
    db: &Daily0060Db,
    f: &SqlPaper,
    target: Uuid,
    settlement: &Value,
    valuation: &Value,
) -> Daily0060Result {
    let before = economic_image(db, f.account).await?;
    let pool = worker_without_actor(db, 1).await?;
    let result: Daily0060Result = async {
        let mut tx = pool.begin().await?;
        let foreign = Uuid::new_v4().to_string();
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(&foreign)
            .execute(&mut *tx)
            .await?;
        let prior = Some(foreign);
        let replay: Value =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_settlement_v1($1,1,$2,$3)")
                .bind(target)
                .bind(settlement)
                .bind(&f.digest)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(replay["outcome"], "REPLAY");
        assert_actor(&mut tx, &prior).await?;
        let replay: Value =
            sqlx::query_scalar("SELECT commit_owner_beta_paper_valuation_v1($1,1,$2,$3)")
                .bind(target)
                .bind(valuation)
                .bind(&f.digest)
                .fetch_one(&mut *tx)
                .await?;
        assert_eq!(replay["outcome"], "REPLAY");
        assert_actor(&mut tx, &prior).await?;
        let mut changed = settlement.clone();
        changed["calculation_sha256"] = json!(hash('9'));
        sqlx::query("SAVEPOINT actor_error")
            .execute(&mut *tx)
            .await?;
        let error = sqlx::query_scalar::<_, Value>(
            "SELECT commit_owner_beta_paper_settlement_v1($1,1,$2,$3)",
        )
        .bind(target)
        .bind(&changed)
        .bind(&f.digest)
        .fetch_one(&mut *tx)
        .await;
        actor_error(&mut tx, error, "23505", &prior).await?;
        let mut changed = valuation.clone();
        changed["calculation_sha256"] = json!(hash('9'));
        sqlx::query("SAVEPOINT actor_error")
            .execute(&mut *tx)
            .await?;
        let error = sqlx::query_scalar::<_, Value>(
            "SELECT commit_owner_beta_paper_valuation_v1($1,1,$2,$3)",
        )
        .bind(target)
        .bind(&changed)
        .bind(&f.digest)
        .fetch_one(&mut *tx)
        .await;
        actor_error(&mut tx, error, "23505", &prior).await?;
        // Commit the replay transaction so a rollback cannot conceal mutations.
        tx.commit().await?;
        Ok(())
    }
    .await;
    pool.close().await;
    result?;
    assert_eq!(
        before,
        economic_image(db, f.account).await?,
        "terminal readback must not mutate ledger, state, events, outbox or equity"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_worker_actor_scope_restores_unset_empty_and_foreign_actor() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, packet) = preview_packet(&db, &f, "MANUAL", false).await?;
        let (target, auto_packet) = auto_target(&db, &f, false).await?;
        let auto_preview = Uuid::parse_str(auto_packet["preview_id"].as_str().ok_or("AUTO preview")?)?;
        let foreign = Uuid::new_v4().to_string();
        for actor in [None, Some(""), Some(foreign.as_str())] {
            // Separate actual worker logins avoid inheriting an earlier function's
            // custom-GUC placeholder or the predecessor fixture's Owner actor.
            let pool = worker_without_actor(&db, 1).await?;
            let case: Daily0060Result = async {
                let mut tx = pool.begin().await?;
                if let Some(actor) = actor {
                    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(actor).execute(&mut *tx).await?;
                }
                let prior: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *tx).await?;
                sqlx::query("UPDATE jobs SET locked_at=clock_timestamp() WHERE id=$1").bind(job).execute(&mut *tx).await?;

                let snapshot: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)").bind(preview).bind(job).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(snapshot["outcome"], "OK");
                assert_eq!(snapshot["data"]["owner_user_id"], db.owner_id.to_string());
                assert_eq!(snapshot["data"]["account_id"], f.account.to_string());
                assert_actor(&mut tx, &prior).await?;
                let missing: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)").bind(Uuid::new_v4()).bind(job).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(missing["outcome"], "NOT_FOUND");
                assert_actor(&mut tx, &prior).await?;
                let stale: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)").bind(preview).bind(Uuid::new_v4()).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(stale["outcome"], "STALE");
                assert_actor(&mut tx, &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT snapshot_owner_beta_paper_preview_v1($1,$2,$3)").bind(preview).bind(job).bind(hash('9')).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;

                let mut wrong_attempt = packet.clone();
                wrong_attempt["attempt_id"] = json!(Uuid::new_v4().to_string());
                assert_eq!(publish_in(&mut tx, &f, preview, job, &wrong_attempt).await?["outcome"], "STALE");
                assert_actor(&mut tx, &prior).await?;
                let mut bad_result = packet.clone();
                bad_result["result"]["plan_status"] = json!("INVALID");
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT publish_owner_beta_paper_preview_v1($1,$2,$3,$4)").bind(preview).bind(job).bind(&bad_result).bind(&f.digest).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "23514", &prior).await?;

                let failure = json!({"code":"SYNTHETIC","message":"actor scope rollback fixture","attempt_id":packet["attempt_id"],"attempt_no":packet["attempt_no"],"worker_id":packet["worker_id"],"process_config_sha256":f.digest});
                let mut wrong_attempt = failure.clone();
                wrong_attempt["attempt_id"] = json!(Uuid::new_v4().to_string());
                let stale: Value = sqlx::query_scalar("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)").bind(preview).bind(job).bind(&wrong_attempt).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(stale["outcome"], "STALE");
                assert_actor(&mut tx, &prior).await?;
                sqlx::query("SAVEPOINT keep_preview").execute(&mut *tx).await?;
                let failed: Value = sqlx::query_scalar("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)").bind(preview).bind(job).bind(&failure).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(failed["outcome"], "OK");
                assert_actor(&mut tx, &prior).await?;
                sqlx::query("ROLLBACK TO SAVEPOINT keep_preview").execute(&mut *tx).await?;
                sqlx::query("RELEASE SAVEPOINT keep_preview").execute(&mut *tx).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)").bind(preview).bind(job).bind(&failure).bind(hash('9')).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;

                let snapshot: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,1,$2)").bind(target).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(snapshot["outcome"], "WAITING_SESSION");
                assert_eq!(snapshot["data"]["owner_user_id"], db.owner_id.to_string());
                assert_actor(&mut tx, &prior).await?;
                let missing: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,1,$2)").bind(Uuid::new_v4()).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(missing["outcome"], "NOT_FOUND");
                assert_actor(&mut tx, &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT snapshot_owner_beta_paper_target_v1($1,1,$2)").bind(target).bind(hash('9')).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;

                let wait: Value = sqlx::query_scalar("SELECT record_owner_beta_paper_wait_v1($1,1,'WAITING_PRICE',$2,$3)").bind(target).bind(hash('a')).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(wait["outcome"], "OK");
                assert_actor(&mut tx, &prior).await?;
                let missing: Value = sqlx::query_scalar("SELECT record_owner_beta_paper_wait_v1($1,1,'WAITING_PRICE',$2,$3)").bind(Uuid::new_v4()).bind(hash('a')).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(missing["outcome"], "NOT_FOUND");
                assert_actor(&mut tx, &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT record_owner_beta_paper_wait_v1($1,1,'INVALID',$2,$3)").bind(target).bind(hash('a')).bind(&f.digest).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "23514", &prior).await?;

                let scan: Value = sqlx::query_scalar("SELECT scan_owner_beta_paper_targets_v1(20,$1)").bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(scan["outcome"], "OK");
                assert_actor(&mut tx, &prior).await?;
                let queue: Value = sqlx::query_scalar("SELECT queue_owner_beta_paper_target_v1($1,1,$2)").bind(f.witness.run).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(queue["outcome"], "REPLAY");
                assert_actor(&mut tx, &prior).await?;
                let apply: Value = sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)").bind(auto_preview).bind(&f.digest).fetch_one(&mut *tx).await?;
                assert_eq!(apply["outcome"], "REPLAY");
                assert_actor(&mut tx, &prior).await?;

                // Adjacent generic worker entrypoints must reject beta identity
                // without relying on the caller actor or exposing tenant rows.
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT to_jsonb(p) FROM snapshot_paper_rebalance_preview($1,$2,current_date) p").bind(preview).bind(job).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT to_jsonb(fail_paper_rebalance_preview($1,$2,'{}'))").bind(preview).bind(job).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT to_jsonb(publish_paper_rebalance_preview($1,$2,0,$3,'KRX_ETF_DEFAULT',1,current_date,$4,'[]','{}'))").bind(preview).bind(job).bind("a".repeat(64)).bind("b".repeat(64)).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;
                sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
                let error = sqlx::query_scalar::<_, Value>("SELECT to_jsonb(p) FROM preflight_paper_target($1,$2) p").bind(target).bind(db.owner_id).fetch_one(&mut *tx).await;
                actor_error(&mut tx, error, "42501", &prior).await?;

                assert_eq!(publish_in(&mut tx, &f, preview, job, &packet).await?["outcome"], "OK");
                assert_actor(&mut tx, &prior).await?;
                tx.rollback().await?;
                Ok(())
            }.await;
            pool.close().await;
            case?;
        }
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_trade_then_unchanged_no_trade_settlement_valuation_atomic_replay()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        super::daily_0060_history(&db).await?;
        let f=enabled_paper(&db,true).await?;
        let generic:Uuid=sqlx::query_scalar("INSERT INTO accounts(owner_user_id,account_type,name,currency,status,initial_cash,cost_profile_id,cost_profile_version) VALUES($1,'PAPER','generic sentinel','KRW','ACTIVE',5000,'KRX_ETF_DEFAULT',1) RETURNING id").bind(db.owner_id).fetch_one(&db.app).await?;
        sqlx::query("INSERT INTO cash_ledger(account_id,owner_user_id,seq,event_type,amount,balance,currency,ts) VALUES($1,$2,1,'DEPOSIT',5000,5000,'KRW',clock_timestamp())").bind(generic).bind(db.owner_id).execute(&db.app).await?;
        let preserved_sql="SELECT jsonb_build_object('account',(SELECT to_jsonb(a) FROM accounts a WHERE id=$1),'cash',(SELECT jsonb_agg(to_jsonb(c) ORDER BY seq) FROM cash_ledger c WHERE account_id=$1),'r4_runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM owner_beta_recommendation_runs r WHERE trigger_kind<>'DAILY'),'r4_items',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.id) FROM owner_beta_recommendation_items i JOIN owner_beta_recommendation_runs r ON r.id=i.recommendation_run_id WHERE r.trigger_kind<>'DAILY'))";
        let preserved:Value=sqlx::query_scalar(preserved_sql).bind(generic).fetch_one(&db.owner).await?;
        let (target,first_preview)=auto_target(&db,&f,true).await?;
        assert_eq!(economic_counts(&db,f.account).await?,(0,0,1,0,0),"T apply cannot fill or value");
        assert_eq!(target_snapshot(&db,&f,target).await?["outcome"],"WAITING_SESSION");
        let waiting:Value=sqlx::query_scalar("SELECT record_owner_beta_paper_wait_v1($1,1,'WAITING_PRICE',$2,$3)").bind(target).bind(hash('a')).bind(&f.digest).fetch_one(&db.worker).await?;assert_eq!(waiting["outcome"],"OK");
        let effective:String=sqlx::query_scalar("SELECT effective_date::text FROM pending_targets WHERE id=$1").bind(target).fetch_one(&db.owner).await?;
        clock.set(initial+86400).await?;
        let today:String=sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date::text").fetch_one(&db.owner).await?;assert_eq!(today,effective,"actual PG clock must advance to E");
        let e=seed_witness(&db,f.config,&f.cohort,&today,Some(&f.witness.admission),true).await?;
        let wire=settlement_packet(&db,&f,target,&e.admission,true).await?;
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        let before=economic_image(&db,f.account).await?;
        denied(settle(&db,&f,target,&wire).await,"42501");
        assert_eq!(before,economic_image(&db,f.account).await?,"revoked rights must deny uncommitted settlement without mutation");
        sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        // Synthetic current-binding rotation tests the fence; this is not a
        // protected runtime/configuration or operator activation proof.
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding_sha256=$1").bind(hash('9')).execute(&db.owner).await?;
        let before=economic_image(&db,f.account).await?;
        denied(settle(&db,&f,target,&wire).await,"42501");
        assert_eq!(before,economic_image(&db,f.account).await?,"old binding must deny new ledger economics");
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding_sha256=$1").bind(&f.digest).execute(&db.owner).await?;
        for field in ["lineage_id","quantity","cash_after","execution_admission_sha256"] {
            let mut invalid=wire.clone();match field {
                "lineage_id"=>invalid[field]=json!(Uuid::new_v4().to_string()),
                "quantity"=>invalid["orders"][0][field]=json!("10.0"),
                "execution_admission_sha256"=>invalid[field]=json!(f.witness.admission),
                _=>invalid[field]=json!("9898901.0000"),
            }
            denied(settle(&db,&f,target,&invalid).await,"23514");
            assert_eq!(economic_counts(&db,f.account).await?,(0,0,1,0,0),"late validation failure must roll back all economics");
        }
        let mut tx=db.worker.begin().await?;
        let trial:Value=sqlx::query_scalar("SELECT commit_owner_beta_paper_settlement_v1($1,1,$2,$3)").bind(target).bind(&wire).bind(&f.digest).fetch_one(&mut *tx).await?;assert_eq!(trial["outcome"],"OK");tx.rollback().await?;
        assert_eq!(economic_counts(&db,f.account).await?,(0,0,1,0,0));
        let (a,b)=tokio::join!(settle(&db,&f,target,&wire),settle(&db,&f,target,&wire));once_and_replay(&a?,&b?);
        assert_eq!(economic_counts(&db,f.account).await?,(1,1,2,1,0));
        let position:(String,Option<String>)=sqlx::query_as("SELECT quantity::text,avg_price::text FROM positions WHERE account_id=$1 AND instrument_id='069500.KRX'").bind(f.account).fetch_one(&db.owner).await?;assert_eq!(position,("10.0000".into(),None),"shared persistence updates quantity without inventing average cost");
        let readback=target_snapshot(&db,&f,target).await?;assert_eq!(readback["data"]["receipt_json"]["settlement"],wire);
        assert_eq!(readback["data"]["ledger_after_sha256"],format!("sha256:{}",readback["data"]["account_state_sha256"].as_str().unwrap()));
        let mut changed=wire.clone();changed["calculation_sha256"]=json!(hash('f'));denied(settle(&db,&f,target,&changed).await,"23505");
        let valuation=valuation_packet(&db,&f,target,&e.admission,"10100.0000","9999900.0000","101000.0000").await?;
        let mut invalid=valuation.clone();invalid["closes"][0]["raw_close"]=json!("10100");denied(valuate(&db,&f,target,&invalid).await,"23514");
        let mut tx=db.worker.begin().await?;
        let v:Value=sqlx::query_scalar("SELECT commit_owner_beta_paper_valuation_v1($1,1,$2,$3)").bind(target).bind(&valuation).bind(&f.digest).fetch_one(&mut *tx).await?;assert_eq!(v["outcome"],"OK");tx.rollback().await?;assert_eq!(economic_counts(&db,f.account).await?,(1,1,2,1,0));
        let (a,b)=tokio::join!(valuate(&db,&f,target,&valuation),valuate(&db,&f,target,&valuation));once_and_replay(&a?,&b?);
        let readback=target_snapshot(&db,&f,target).await?;assert_eq!(readback["data"]["session_state"],"VALUED");assert_eq!(readback["data"]["receipt_json"]["valuation"],valuation);
        let second=SqlPaper{account:f.account,config:f.config,cohort:f.cohort.clone(),digest:f.digest.clone(),witness:e.clone()};
        let (target2,second_preview)=auto_target(&db,&second,false).await?;
        assert_ne!(first_preview["target_snapshot_sha256"],second_preview["target_snapshot_sha256"],"daily W identity varies");assert_eq!(first_preview["signal_allocation_sha256"],second_preview["signal_allocation_sha256"],"unchanged signal ignores daily W");
        assert_eq!(economic_counts(&db,f.account).await?,(1,1,2,1,1));
        clock.set(initial+172800).await?;
        let day:String=sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date::text").fetch_one(&db.owner).await?;
        let next=seed_witness(&db,f.config,&f.cohort,&day,Some(&e.admission),true).await?;
        let no_trade=settlement_packet(&db,&f,target2,&next.admission,false).await?;
        let receipt=settle(&db,&f,target2,&no_trade).await?;assert_eq!(receipt["outcome"],"OK");
        assert_eq!(economic_counts(&db,f.account).await?,(1,1,2,2,1),"unchanged signal writes zero new economics and one outbox");
        let v=valuation_packet(&db,&f,target2,&next.admission,"10200.0000","10000900.0000","102000.0000").await?;
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        let before=economic_image(&db,f.account).await?;
        denied(valuate(&db,&f,target2,&v).await,"42501");
        assert_eq!(before,economic_image(&db,f.account).await?,"new equity valuation still requires current rights");
        terminal_readback_without_mutation(&db,&f,target,&wire,&valuation).await?;
        sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding_sha256=$1").bind(hash('9')).execute(&db.owner).await?;
        let before=economic_image(&db,f.account).await?;
        denied(valuate(&db,&f,target2,&v).await,"42501");
        assert_eq!(before,economic_image(&db,f.account).await?,"old binding must deny uncommitted valuation");
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding_sha256=$1").bind(&f.digest).execute(&db.owner).await?;
        assert_eq!(valuate(&db,&f,target2,&v).await?["outcome"],"OK");assert_eq!(economic_counts(&db,f.account).await?,(1,1,2,2,2));
        let state:Value=sqlx::query_scalar("SELECT read_owner_beta_paper_state_v1()").fetch_one(&db.app).await?;assert_eq!(state["data"]["sessions"].as_array().unwrap().len(),2);
        let revoked:Value=sqlx::query_scalar("SELECT record_owner_beta_paper_consent_v1($1,1,'owner-beta-paper-v1-draft-1',$2,false,'revoke-after-valuations')").bind(f.account).bind(financial_contract()["contract_sha256"].as_str().unwrap()).fetch_one(&db.app).await?;
        assert_eq!(revoked["control_epoch"],2);
        sqlx::query("UPDATE owner_beta_paper_control SET process_binding_sha256=$1").bind(hash('9')).execute(&db.owner).await?;
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        terminal_readback_without_mutation(&db,&f,target,&wire,&valuation).await?;
        terminal_readback_without_mutation(&db,&f,target2,&no_trade,&v).await?;
        let before=economic_image(&db,f.account).await?;
        denied(sqlx::query_scalar::<_,Value>("SELECT queue_owner_beta_paper_target_v1($1,2,$2)").bind(next.run).bind(hash('9')).fetch_one(&db.worker).await,"40001");
        assert_eq!(before,economic_image(&db,f.account).await?,"DRAINING readback must not authorize a new target");
        let after:Value=sqlx::query_scalar(preserved_sql).bind(generic).fetch_one(&db.owner).await?;assert_eq!(preserved,after,"generic Paper and R4 rows must remain byte-for-byte equal as JSON");
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_ready_auto_retry_denies_revoked_admission_after_earlier_valuation()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        emit_r5_paper_test_witness("READY_SETUP", "ENTERED", None, None, None, None, None);
        super::daily_0060_history(&db).await?;
        let f = enabled_paper(&db, true).await?;
        emit_r5_paper_test_witness("READY_T1_TARGET", "ENTERED", None, None, None, None, None);
        let (t1, _) = auto_target(&db, &f, true).await?;
        let e1_date: String = sqlx::query_scalar(
            "SELECT effective_date::text FROM pending_targets WHERE id=$1",
        )
        .bind(t1)
        .fetch_one(&db.owner)
        .await?;

        clock.set(initial + 86400).await?;
        let actual_e1_date: String = sqlx::query_scalar(
            "SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date::text",
        )
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(actual_e1_date, e1_date, "T1 must settle on its effective date");
        emit_r5_paper_test_witness("READY_T1_SETTLE", "ENTERED", None, None, None, None, None);
        let e1 = seed_witness(
            &db,
            f.config,
            &f.cohort,
            &actual_e1_date,
            Some(&f.witness.admission),
            true,
        )
        .await?;
        assert_eq!(e1.session, e1_date);
        let t1_settlement = settlement_packet(&db, &f, t1, &e1.admission, true).await?;
        assert_eq!(settle(&db, &f, t1, &t1_settlement).await?["outcome"], "OK");
        let t1_after_settlement = target_snapshot(&db, &f, t1).await?;
        assert_eq!(t1_after_settlement["data"]["session_state"], "SETTLED_TRADE");
        let account_after_t1_settlement = economic_image(&db, f.account).await?;

        let second = SqlPaper {
            account: f.account,
            config: f.config,
            cohort: f.cohort.clone(),
            digest: f.digest.clone(),
            witness: e1.clone(),
        };
        emit_r5_paper_test_witness("READY_T2_PREVIEW", "ENTERED", None, None, None, None, None);
        let (t2_preview, t2_job, t2_packet) = preview_packet(&db, &second, "AUTO", false).await?;
        let t2_lineage = Uuid::parse_str(
            t2_packet["lineage_id"]
                .as_str()
                .ok_or("T2 lineage")?,
        )?;
        let e2_date: String = sqlx::query_scalar(
            "SELECT effective_session::text FROM owner_beta_paper_lineage WHERE id=$1",
        )
        .bind(t2_lineage)
        .fetch_one(&db.owner)
        .await?;
        assert!(e1_date < e2_date, "T2 must be later than the settled T1 session");
        assert_eq!(
            t2_packet["account_state_sha256"],
            t1_after_settlement["data"]["account_state_sha256"],
            "T2 preview must capture the account after T1 settlement"
        );

        emit_r5_paper_test_witness("READY_T2_WAIT", "ENTERED", None, None, None, None, None);
        let mut publish = db.worker.begin().await?;
        let waiting = publish_in(&mut publish, &second, t2_preview, t2_job, &t2_packet).await?;
        assert_eq!(waiting["outcome"], "WAITING_EARLIER_SESSION");
        finish_preview(&mut publish, t2_job).await?;
        publish.commit().await?;
        let ready_preview: Value = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1",
        )
        .bind(t2_preview)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(ready_preview["status"], "READY");
        assert!(ready_preview["pending_target_id"].is_null());
        let t2_target_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id=$2",
        )
        .bind(f.account)
        .bind(t2_lineage)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(t2_target_count, 0, "waiting T2 must not create its target");
        assert_eq!(
            account_after_t1_settlement,
            economic_image(&db, f.account).await?,
            "waiting T2 publication must not change the settled account"
        );

        // Match the existing trade regression's ledger-consistent valuation.
        emit_r5_paper_test_witness("READY_T1_VALUE", "ENTERED", None, None, None, None, None);
        let t1_valuation = valuation_packet(
            &db,
            &f,
            t1,
            &e1.admission,
            "10100.0000",
            "9999900.0000",
            "101000.0000",
        )
        .await?;
        assert_eq!(valuate(&db, &f, t1, &t1_valuation).await?["outcome"], "OK");
        let t1_after_valuation = target_snapshot(&db, &f, t1).await?;
        assert_eq!(t1_after_valuation["data"]["session_state"], "VALUED");
        let earlier_targets: Vec<Uuid> = sqlx::query_scalar(
            "SELECT s.pending_target_id FROM owner_beta_paper_sessions s JOIN pending_targets t ON t.id=s.pending_target_id WHERE t.account_id=$1 AND t.effective_date<$2::text::date ORDER BY t.effective_date",
        )
        .bind(f.account)
        .bind(&e2_date)
        .fetch_all(&db.owner)
        .await?;
        assert!(earlier_targets.contains(&t1), "T1 must be an earlier completed session");
        for earlier_target in earlier_targets {
            assert_eq!(
                target_snapshot(&db, &f, earlier_target).await?["data"]["session_state"],
                "VALUED",
                "T2 retry must have no earlier unfinished session"
            );
        }

        let before_control: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE c.account_id=$1",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        let before_preview: Value = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1",
        )
        .bind(t2_preview)
        .fetch_one(&db.owner)
        .await?;
        let before_economics = economic_image(&db, f.account).await?;
        let before_counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;

        emit_r5_paper_test_witness("READY_TRIAL_APPLY", "ENTERED", None, None, None, None, None);
        let preview_id = t2_preview.to_string();
        let owner_id = db.owner_id.to_string();
        let account_id = f.account.to_string();
        let trial_readback_sql = format!(
            r#"CREATE FUNCTION public.r5_paper_ci17_trial_readback()
               RETURNS TABLE(preview_json jsonb, target_id uuid, effective_date text,
                             target_count bigint, session_count bigint)
               LANGUAGE plpgsql SECURITY DEFINER
               SET search_path = pg_catalog, pg_temp
               AS $body$
               DECLARE
                   caller_actor text;
                   preview_value jsonb;
                   target_value uuid;
                   effective_value text;
                   target_rows bigint;
                   session_rows bigint;
               BEGIN
                   caller_actor := pg_catalog.current_setting('app.actor_user_id', true);
                   PERFORM pg_catalog.set_config('app.actor_user_id', '{owner_id}', true);
                   BEGIN
                       SELECT pg_catalog.to_jsonb(p), p.pending_target_id
                         INTO preview_value, target_value
                         FROM public.paper_rebalance_previews AS p
                        WHERE p.id = '{preview_id}'::pg_catalog.uuid
                          AND p.owner_user_id = '{owner_id}'::pg_catalog.uuid
                          AND p.account_id = '{account_id}'::pg_catalog.uuid;
                       IF preview_value IS NULL OR target_value IS NULL THEN
                           RAISE EXCEPTION USING ERRCODE = 'P0002';
                       END IF;
                       SELECT t.effective_date::text INTO effective_value
                         FROM public.pending_targets AS t
                        WHERE t.id = target_value
                          AND t.account_id = '{account_id}'::pg_catalog.uuid;
                       IF NOT FOUND THEN
                           RAISE EXCEPTION USING ERRCODE = 'P0002';
                       END IF;
                       SELECT pg_catalog.count(*) INTO target_rows
                         FROM public.pending_targets AS t
                        WHERE t.id = target_value
                          AND t.account_id = '{account_id}'::pg_catalog.uuid;
                       SELECT pg_catalog.count(*) INTO session_rows
                         FROM public.owner_beta_paper_sessions AS s
                        WHERE s.pending_target_id = target_value
                          AND s.account_id = '{account_id}'::pg_catalog.uuid;
                   EXCEPTION WHEN OTHERS THEN
                       PERFORM pg_catalog.set_config('app.actor_user_id',
                                                     COALESCE(caller_actor, ''), true);
                       RAISE;
                   END;
                   PERFORM pg_catalog.set_config('app.actor_user_id',
                                                 COALESCE(caller_actor, ''), true);
                   RETURN QUERY SELECT preview_value, target_value, effective_value,
                                       target_rows, session_rows;
               END
               $body$"#
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(trial_readback_sql))
            .execute(&db.owner)
            .await?;
        sqlx::query("REVOKE ALL ON FUNCTION public.r5_paper_ci17_trial_readback() FROM PUBLIC")
            .execute(&db.owner)
            .await?;
        sqlx::query("GRANT EXECUTE ON FUNCTION public.r5_paper_ci17_trial_readback() TO worker")
            .execute(&db.owner)
            .await?;

        let mut trial = db.worker.begin().await?;
        let trial_result: Value = sqlx::query_scalar(
            "SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)",
        )
        .bind(t2_preview)
        .bind(&f.digest)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(trial_result["outcome"], "OK", "unrevoked READY retry reaches apply");
        emit_r5_paper_test_witness("READY_TRIAL_READBACK", "ENTERED", None, None, None, None, None);
        let trial_actor_before = sqlx::query_scalar::<_, Option<String>>(
            "SELECT pg_catalog.current_setting('app.actor_user_id', true)",
        )
        .fetch_one(&mut *trial)
        .await?;
        let (applied_preview, trial_target, trial_effective, trial_target_rows, trial_session_rows):
            (Value, Uuid, String, i64, i64) = sqlx::query_as(
                "SELECT preview_json, target_id, effective_date, target_count, session_count FROM public.r5_paper_ci17_trial_readback()",
            )
            .fetch_one(&mut *trial)
            .await?;
        assert_actor(&mut trial, &trial_actor_before).await?;
        let direct_worker_rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.paper_rebalance_previews WHERE id=$1),(SELECT count(*) FROM public.pending_targets WHERE id=$2)",
        )
        .bind(t2_preview)
        .bind(trial_target)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(
            direct_worker_rows,
            (0, 0),
            "the bounded fixture readback must not broaden worker table visibility"
        );
        assert_eq!(applied_preview["status"], "APPLIED");
        assert_eq!(
            applied_preview["pending_target_id"].as_str(),
            Some(trial_target.to_string().as_str()),
            "readback target is the exact preview's referenced target"
        );
        assert_ne!(trial_target, t1);
        assert_eq!(trial_effective, e2_date, "trial retry creates the E2 target");
        assert_eq!((trial_target_rows, trial_session_rows), (1, 1), "trial target has its real session row");
        trial.rollback().await?;

        let rolled_back_target_rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE pending_target_id=$1)",
        )
        .bind(trial_target)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(rolled_back_target_rows, (0, 0));
        assert_eq!(
            sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1",
            )
            .bind(t2_preview)
            .fetch_one(&db.owner)
            .await?,
            before_preview,
            "trial rollback restores the READY preview"
        );
        assert_eq!(
            sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE c.account_id=$1",
            )
            .bind(f.account)
            .fetch_one(&db.owner)
            .await?,
            before_control
        );
        assert_eq!(economic_image(&db, f.account).await?, before_economics);
        let after_trial_counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(after_trial_counts, before_counts);
        let worker_preview: Option<Value> = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM public.paper_rebalance_previews AS p WHERE p.id=$1",
        )
        .bind(t2_preview)
        .fetch_optional(&db.worker)
        .await?;
        assert!(worker_preview.is_none(), "worker beta preview remains invisible");

        emit_r5_paper_test_witness("READY_REVOKE_RETRY", "ENTERED", None, None, None, None, None);
        let revoked: bool = sqlx::query_scalar(
            "SELECT revoke_owner_beta_daily_input($1,$2)",
        )
        .bind(&e1.admission)
        .bind(hash('f'))
        .fetch_one(&db.owner)
        .await?;
        assert!(revoked, "the T2 signal admission must be revoked");
        let before_retry = json!({
            "control": sqlx::query_scalar::<_, Value>("SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE c.account_id=$1").bind(f.account).fetch_one(&db.owner).await?,
            "preview": sqlx::query_scalar::<_, Value>("SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1").bind(t2_preview).fetch_one(&db.owner).await?,
            "economics": economic_image(&db, f.account).await?,
            "target_session_counts": sqlx::query_as::<_, (i64, i64)>("SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)").bind(f.account).fetch_one(&db.owner).await?
        });
        denied(
            sqlx::query_scalar::<_, Value>(
                "SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)",
            )
            .bind(t2_preview)
            .bind(&f.digest)
            .fetch_one(&db.worker)
            .await,
            "42501",
        );
        let after_retry = json!({
            "control": sqlx::query_scalar::<_, Value>("SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE c.account_id=$1").bind(f.account).fetch_one(&db.owner).await?,
            "preview": sqlx::query_scalar::<_, Value>("SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1").bind(t2_preview).fetch_one(&db.owner).await?,
            "economics": economic_image(&db, f.account).await?,
            "target_session_counts": sqlx::query_as::<_, (i64, i64)>("SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)").bind(f.account).fetch_one(&db.owner).await?
        });
        assert_eq!(after_retry, before_retry, "revoked retry must leave all captured state unchanged");
        assert_eq!(after_retry["preview"]["status"], "READY");
        assert!(after_retry["preview"]["pending_target_id"].is_null());
        let target_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id=$2",
        )
        .bind(f.account)
        .bind(t2_lineage)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(target_count, 0, "revoked T2 retry creates no E2 target");
        assert_eq!(
            target_snapshot(&db, &f, t1).await?["data"]["session_state"],
            "VALUED",
            "revocation must preserve completed T1 history"
        );
        Ok(())
    }
    .await;
    let close = db.close().await;
    let reset_clock = clock.set(0).await;
    close?;
    reset_clock?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_settled_cross_midnight_recovery_values_once_without_reexecuting()
-> Daily0060Result {
    for trade in [true, false] {
        let db = isolated_db().await?;
        let clock = FixtureClock::new().await?;
        let initial = clock.after_cutoff().await?;
        let result: Daily0060Result = async {
            let f = enabled_paper(&db, trade).await?;
            let (target, _) = auto_target(&db, &f, trade).await?;
            clock.set(initial + 86400).await?;
            let day: String = sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date::text").fetch_one(&db.owner).await?;
            let e = seed_witness(&db, f.config, &f.cohort, &day, Some(&f.witness.admission), trade).await?;
            let settlement = settlement_packet(&db, &f, target, &e.admission, trade).await?;
            assert_eq!(settle(&db, &f, target, &settlement).await?["outcome"], "OK");
            let valuation = valuation_packet(&db, &f, target, &e.admission, "10100.0000", if trade { "9999900.0000" } else { "10000000.0000" }, if trade { "101000.0000" } else { "0.0000" }).await?;
            let settled = economic_image(&db, f.account).await?;
            assert_eq!(settled["sessions"][0]["state"], if trade { "SETTLED_TRADE" } else { "SETTLED_NO_TRADE" });
            assert_eq!(settled["targets"][0]["status"], if trade { "EXECUTED" } else { "SKIPPED" });

            // Simulate restart after the committed E settlement, before valuation.
            // Actual process time changes; no immutable lineage is backdated.
            clock.set(initial + 172800).await?;
            assert_eq!(valuate(&db, &f, target, &valuation).await?["outcome"], "WAITING_SESSION");
            assert_eq!(settled, economic_image(&db, f.account).await?, "late valuation wait is read-only");
            let recovery = json!({"schema":"owner-beta-paper-recovery-v1","pending_target_id":target.to_string(),"effective_session":day,"reason_code":"RECOVERY_REQUIRED","decision_sha256":hash('8')});
            let mut wrong = recovery.clone();
            wrong["pending_target_id"] = json!(Uuid::new_v4().to_string());
            denied(sqlx::query("SELECT transition_owner_beta_paper_v1(1,'RECOVERY',$1,'wrong-recovery-target')").bind(&wrong).execute(&db.owner).await, "23514");
            let today: String = sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date::text").fetch_one(&db.owner).await?;
            let mut wrong = recovery.clone();
            wrong["effective_session"] = json!(today);
            denied(sqlx::query("SELECT transition_owner_beta_paper_v1(1,'RECOVERY',$1,'wrong-recovery-date')").bind(&wrong).execute(&db.owner).await, "23514");
            sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
            denied(sqlx::query("SELECT transition_owner_beta_paper_v1(1,'RECOVERY',$1,'revoked-recovery')").bind(&recovery).execute(&db.owner).await, "42501");
            sqlx::query("UPDATE data_entitlements SET status='ACTIVE' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
            let binding: Value = sqlx::query_scalar("SELECT process_binding FROM owner_beta_paper_control").fetch_one(&db.owner).await?;
            let mut wrong_binding = binding.clone();
            wrong_binding["account_id"] = json!(Uuid::new_v4().to_string());
            sqlx::query("UPDATE owner_beta_paper_control SET process_binding=$1").bind(&wrong_binding).execute(&db.owner).await?;
            denied(sqlx::query("SELECT transition_owner_beta_paper_v1(1,'RECOVERY',$1,'wrong-binding-recovery')").bind(&recovery).execute(&db.owner).await, "42501");
            sqlx::query("UPDATE owner_beta_paper_control SET process_binding=$1").bind(&binding).execute(&db.owner).await?;
            assert_eq!(settled, economic_image(&db, f.account).await?, "rejected recovery must retain all settlement state");

            // The lifecycle role also starts with no app actor; test exact
            // restoration on success, replay and errors for all three contexts.
            transition_with_actor_checks(&db, 1, "RECOVERY", &recovery, "recover-valuation").await?;
            let authorized = economic_image(&db, f.account).await?;
            assert_eq!(settled["sessions"], authorized["sessions"], "recovery must preserve settled origin/state/receipt and ledger fingerprints");
            assert_eq!(authorized["control_events"].as_array().unwrap().len(), settled["control_events"].as_array().unwrap().len() + 1);
            assert_eq!(authorized["session_events"].as_array().unwrap().len(), settled["session_events"].as_array().unwrap().len() + 1);
            let (a, b) = tokio::join!(valuate(&db, &f, target, &valuation), valuate(&db, &f, target, &valuation));
            once_and_replay(&a?, &b?);
            let valued = economic_image(&db, f.account).await?;
            for key in ["account", "binding", "cash", "positions", "orders", "fills", "targets", "outbox"] {
                assert_eq!(settled[key], valued[key], "valuation recovery must not reexecute {key}");
            }
            assert_eq!(valued["sessions"][0]["state"], "VALUED");
            assert_eq!(valued["sessions"][0]["processing_origin"], "AUTO");
            assert_eq!(valued["sessions"][0]["receipt_json"]["settlement"], settlement);
            assert_eq!(valued["equity"].as_array().unwrap().len(), 1);
            terminal_readback_without_mutation(&db, &f, target, &settlement, &valuation).await?;
            denied(sqlx::query("SELECT transition_owner_beta_paper_v1(1,'RECOVERY',$1,'recover-already-valued')").bind(&recovery).execute(&db.owner).await, "23514");
            Ok(())
        }.await;
        db.close().await?;
        clock.set(0).await?;
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_manual_apply_concurrency_tokens_stale_publication_and_revocation()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let (preview,job,packet)=preview_packet(&db,&f,"MANUAL",false).await?;
        let mut wrong_claim=packet.clone();wrong_claim["attempt_id"]=json!(Uuid::new_v4().to_string());
        let mut tx=db.worker.begin().await?;assert_eq!(publish_in(&mut tx,&f,preview,job,&wrong_claim).await?["outcome"],"STALE");tx.rollback().await?;
        sqlx::query("UPDATE jobs SET locked_at=clock_timestamp()-interval '61 seconds' WHERE id=$1").bind(job).execute(&db.worker).await?;
        let mut tx=db.worker.begin().await?;assert_eq!(publish_in(&mut tx,&f,preview,job,&packet).await?["outcome"],"STALE");tx.rollback().await?;
        sqlx::query("UPDATE jobs SET locked_at=clock_timestamp() WHERE id=$1").bind(job).execute(&db.worker).await?;
        let failure=json!({"code":"SYNTHETIC","message":"SQL rollback fixture","attempt_id":packet["attempt_id"],"attempt_no":packet["attempt_no"],"worker_id":packet["worker_id"],"process_config_sha256":f.digest});
        let mut tx=db.worker.begin().await?;let failed:Value=sqlx::query_scalar("SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)").bind(preview).bind(job).bind(failure).bind(&f.digest).fetch_one(&mut *tx).await?;assert_eq!(failed["outcome"],"OK");tx.rollback().await?;
        denied(sqlx::query("SELECT snapshot_paper_rebalance_preview($1,$2,current_date)").bind(preview).bind(job).execute(&db.worker).await,"42501");
        let mut stale=packet.clone();stale["account_state_version"]=json!(999);
        let mut tx=db.worker.begin().await?;assert_eq!(publish_in(&mut tx,&f,preview,job,&stale).await?["outcome"],"STALE");tx.rollback().await?;
        let mut tx=db.worker.begin().await?;assert_eq!(publish_in(&mut tx,&f,preview,job,&packet).await?["outcome"],"OK");finish_preview(&mut tx,job).await?;tx.commit().await?;
        let wrong:Value=sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)").bind(preview).bind("f".repeat(64)).bind(&f.digest).fetch_one(&db.app).await?;assert_eq!(wrong["outcome"],"STALE");
        denied(sqlx::query("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)").bind(preview).bind("e".repeat(64)).bind(&f.digest).execute(&db.worker).await,"42501");
        let apply=||sqlx::query_scalar::<_,Value>("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)").bind(preview).bind("e".repeat(64)).bind(&f.digest);
        let (a,b)=tokio::join!(apply().fetch_one(&db.app),apply().fetch_one(&db.app));let(a,b)=(a?,b?);once_and_replay(&a,&b);
        assert_eq!(economic_counts(&db,f.account).await?,(0,0,1,0,0));
        sqlx::query("UPDATE user_strategy_configs SET config_json=config_json||'{\"edited\":true}' WHERE id=$1").bind(f.config).execute(&db.app).await?;
        let state:Value=sqlx::query_scalar("SELECT read_owner_beta_paper_state_v1()").fetch_one(&db.app).await?;assert_eq!(state["data"]["control"]["state"],"BLOCKED");
        let revoked:Value=sqlx::query_scalar("SELECT record_owner_beta_paper_consent_v1($1,1,'owner-beta-paper-v1-draft-1',$2,false,'revoke-pending')").bind(f.account).bind(financial_contract()["contract_sha256"].as_str().unwrap()).fetch_one(&db.app).await?;assert_eq!(revoked["control_epoch"],2);
        let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND status='PENDING'),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)").bind(f.account).fetch_one(&db.owner).await?;assert_eq!(counts,(1,1),"revocation preserves pending/history");
        denied(apply().fetch_one(&db.app).await,"40001");
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_apply_rejects_direct_and_ancestor_revocation_without_mutation()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let (manual,manual_job,manual_packet)=preview_packet(&db,&f,"MANUAL",false).await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,manual,manual_job,&manual_packet).await?["outcome"],"OK");
        finish_preview(&mut tx,manual_job).await?;
        tx.commit().await?;
        let (automatic,automatic_job,automatic_packet)=preview_packet(&db,&f,"AUTO",false).await?;
        let parent:String=sqlx::query_scalar("SELECT parent_admission_sha256 FROM owner_beta_daily_admissions WHERE admission_sha256=$1 AND owner_user_id=$2")
            .bind(&f.witness.admission).bind(db.owner_id).fetch_one(&db.owner).await?;
        assert!(sqlx::query_scalar::<_,bool>("SELECT revoke_owner_beta_daily_input($1,$2)")
            .bind(&f.witness.admission).bind(hash('f')).fetch_one(&db.owner).await?);
        assert!(sqlx::query_scalar::<_,bool>("SELECT revoke_owner_beta_daily_input($1,$2)")
            .bind(parent).bind(hash('e')).fetch_one(&db.owner).await?);
        let before_control:Value=sqlx::query_scalar("SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'")
            .fetch_one(&db.owner).await?;
        let before_counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)")
            .fetch_one(&db.owner).await?;
        denied(sqlx::query("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
            .bind(manual).bind(manual_packet["preview_token"].as_str().ok_or("manual token")?)
            .bind(&f.digest).fetch_one(&db.app).await,"42501");
        let mut tx=db.worker.begin().await?;
        let failed=publish_in(&mut tx,&f,automatic,automatic_job,&automatic_packet).await;
        denied(failed,"42501");
        tx.rollback().await?;
        let after_control:Value=sqlx::query_scalar("SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'")
            .fetch_one(&db.owner).await?;
        let after_counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)")
            .fetch_one(&db.owner).await?;
        assert_eq!(after_control,before_control,"rejected MANUAL/AUTO target creation cannot mutate control");
        assert_eq!(after_counts,before_counts,"revoked READY MANUAL and AUTO publication cannot create target/session rows");
        let statuses:(String,String)=sqlx::query_as("SELECT (SELECT status FROM paper_rebalance_previews WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)")
            .bind(manual).bind(automatic).fetch_one(&db.owner).await?;
        assert_eq!(statuses,("READY".into(),"RUNNING".into()),"failed AUTO publication rolls back its transient READY projection");
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_apply_rejects_ancestor_only_revocation_without_mutation() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (manual, _, manual_packet, automatic, automatic_job, automatic_packet) =
            manual_ready_auto_running(&db, &f).await?;
        let parent: String = sqlx::query_scalar(
            "SELECT parent_admission_sha256 FROM owner_beta_daily_admissions WHERE admission_sha256=$1 AND owner_user_id=$2",
        )
        .bind(&f.witness.admission)
        .bind(db.owner_id)
        .fetch_one(&db.owner)
        .await?;
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT revoke_owner_beta_daily_input($1,$2)",
        )
        .bind(parent)
        .bind(hash('f'))
        .fetch_one(&db.owner)
        .await?);
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT NOT EXISTS(SELECT 1 FROM owner_beta_daily_admission_events WHERE admission_sha256=$1 AND kind='REVOKED')",
        )
        .bind(&f.witness.admission)
        .fetch_one(&db.owner)
        .await?, "only the ancestor is directly revoked in this case");
        assert!(!sqlx::query_scalar::<_, bool>(
            "SELECT daily_0060_admission_usable((SELECT d FROM owner_beta_daily_control d WHERE id='etf11' AND owner_user_id=$1),$2,false)",
        )
        .bind(db.owner_id)
        .bind(&f.witness.admission)
        .fetch_one(&db.owner)
        .await?);

        let control_before: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_before: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)",
        )
        .fetch_one(&db.owner)
        .await?;
        denied(
            sqlx::query("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
                .bind(manual)
                .bind(manual_packet["preview_token"].as_str().ok_or("manual token")?)
                .bind(&f.digest)
                .fetch_one(&db.app)
                .await,
            "42501",
        );
        let mut tx = db.worker.begin().await?;
        denied(
            publish_in(&mut tx, &f, automatic, automatic_job, &automatic_packet).await,
            "42501",
        );
        tx.rollback().await?;
        let control_after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_after: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)",
        )
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(control_after, control_before);
        assert_eq!(counts_after, counts_before);
        let statuses: (String, String) = sqlx::query_as(
            "SELECT (SELECT status FROM paper_rebalance_previews WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)",
        )
        .bind(manual)
        .bind(automatic)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(statuses, ("READY".into(), "RUNNING".into()));
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_apply_rejects_overlapping_source_conflict_without_mutation() -> Daily0060Result
{
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (manual, _, manual_packet, automatic, automatic_job, automatic_packet) =
            manual_ready_auto_running(&db, &f).await?;
        let event = record_source_conflict(&db, &f).await?;
        let overlapping: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM owner_beta_daily_source_events WHERE event_sha256=$1 AND owner_user_id=$2 AND range_start<=(SELECT range_end FROM owner_beta_daily_admissions WHERE admission_sha256=$3) AND range_end>=(SELECT range_start FROM owner_beta_daily_admissions WHERE admission_sha256=$3)",
        )
        .bind(event)
        .bind(db.owner_id)
        .bind(&f.witness.admission)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(overlapping, 1, "the real source-conflict API records an overlapping range");
        assert!(!sqlx::query_scalar::<_, bool>(
            "SELECT daily_0060_admission_usable((SELECT d FROM owner_beta_daily_control d WHERE id='etf11' AND owner_user_id=$1),$2,false)",
        )
        .bind(db.owner_id)
        .bind(&f.witness.admission)
        .fetch_one(&db.owner)
        .await?);
        let control_before: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_before: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)",
        )
        .fetch_one(&db.owner)
        .await?;
        denied(
            sqlx::query("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
                .bind(manual)
                .bind(manual_packet["preview_token"].as_str().ok_or("manual token")?)
                .bind(&f.digest)
                .fetch_one(&db.app)
                .await,
            "42501",
        );
        let mut tx = db.worker.begin().await?;
        denied(
            publish_in(&mut tx, &f, automatic, automatic_job, &automatic_packet).await,
            "42501",
        );
        tx.rollback().await?;
        let control_after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_after: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions)",
        )
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(control_after, control_before);
        assert_eq!(counts_after, counts_before);
        let statuses: (String, String) = sqlx::query_as(
            "SELECT (SELECT status FROM paper_rebalance_previews WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)",
        )
        .bind(manual)
        .bind(automatic)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(statuses, ("READY".into(), "RUNNING".into()));
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_apply_source_lock_is_shared_bounded_and_replay_safe() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, packet) = preview_packet(&db, &f, "MANUAL", false).await?;
        let mut tx = db.worker.begin().await?;
        assert_eq!(
            publish_in(&mut tx, &f, preview, job, &packet).await?["outcome"],
            "OK"
        );
        finish_preview(&mut tx, job).await?;
        tx.commit().await?;

        let mut exclusive = db.owner.begin().await?;
        sqlx::query("SELECT daily_0060_source_lock($1,true)")
            .bind(db.owner_id)
            .execute(&mut *exclusive)
            .await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *exclusive)
            .await?;
        let preview_token = packet["preview_token"].as_str().ok_or("manual token")?;
        let mut apply_stage = "APPLY_POOL_ACQUIRE";
        let mut apply_backend_pid = None;
        let mut advisory_blocked = None;
        emit_r5_paper_test_witness(
            "APPLY_POOL_ACQUIRE",
            "POOL_PENDING",
            None,
            None,
            None,
            None,
            None,
        );
        let bounded = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            let mut connection = db.app.acquire().await?;
            apply_stage = "APPLY_QUERY";
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            apply_backend_pid = Some(pid);
            emit_r5_paper_test_witness(
                "APPLY_QUERY",
                "QUERY_PENDING",
                None,
                Some(pid),
                Some(blocker_pid),
                None,
                None,
            );
            let apply_query = sqlx::query_scalar::<_, Value>(
                "SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)",
            )
            .bind(preview)
            .bind(preview_token)
            .bind(&f.digest)
            .fetch_one(&mut *connection);
            tokio::pin!(apply_query);
            loop {
                tokio::select! {
                    result = &mut apply_query => return result,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                        let observed: Result<bool, sqlx::Error> = sqlx::query_scalar(
                            "SELECT $2 = ANY(pg_catalog.pg_blocking_pids($1)) AND EXISTS (SELECT 1 FROM pg_catalog.pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)",
                        )
                        .bind(pid)
                        .bind(blocker_pid)
                        .fetch_one(&db.owner)
                        .await;
                        if let Ok(true) = observed && advisory_blocked.is_none() {
                            advisory_blocked = Some(true);
                            emit_r5_paper_test_witness(
                                "APPLY_QUERY",
                                "QUERY_PENDING",
                                None,
                                Some(pid),
                                Some(blocker_pid),
                                Some(true),
                                None,
                            );
                        }
                    }
                }
            }
        })
        .await;
        exclusive.rollback().await?;
        let bounded = match bounded {
            Ok(Ok(value)) => {
                emit_r5_paper_test_witness(
                    "APPLY_QUERY",
                    "UPDATE_OK",
                    None,
                    apply_backend_pid,
                    Some(blocker_pid),
                    advisory_blocked,
                    None,
                );
                Ok(value)
            }
            Ok(Err(error)) => {
                if let Some(database_error) = error.as_database_error() {
                    if let Some(code) = database_error.code() {
                        emit_r5_paper_test_witness(
                            apply_stage,
                            "DATABASE_ERROR",
                            Some(code.as_ref()),
                            apply_backend_pid,
                            Some(blocker_pid),
                            advisory_blocked,
                            None,
                        );
                    } else {
                        emit_r5_paper_test_witness(
                            apply_stage,
                            "DATABASE_ERROR",
                            None,
                            apply_backend_pid,
                            Some(blocker_pid),
                            advisory_blocked,
                            None,
                        );
                    }
                } else {
                    emit_r5_paper_test_witness(
                        apply_stage,
                        "NON_DATABASE_ERROR",
                        None,
                        apply_backend_pid,
                        Some(blocker_pid),
                        advisory_blocked,
                        None,
                    );
                }
                Err(error)
            }
            Err(_elapsed) => {
                emit_r5_paper_test_witness(
                    apply_stage,
                    "CLIENT_TIMEOUT",
                    None,
                    apply_backend_pid,
                    Some(blocker_pid),
                    advisory_blocked,
                    None,
                );
                panic!("source-lock wait must terminate within the SQL lock bound");
            }
        };
        denied(bounded, "55P03");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM pending_targets WHERE account_id=$1"
            )
            .bind(f.account)
            .fetch_one(&db.owner)
            .await?,
            0,
            "lock timeout cannot create target/session state"
        );

        let mut applying = db.app.begin().await?;
        let applying_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *applying)
            .await?;
        let applied: Value =
            sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
                .bind(preview)
                .bind(packet["preview_token"].as_str().ok_or("manual token")?)
                .bind(&f.digest)
                .fetch_one(&mut *applying)
                .await?;
        assert_eq!(applied["outcome"], "OK");
        let owner_pool = db.owner.clone();
        let owner_id = db.owner_id;
        let admission = f.witness.admission.clone();
        let evidence = hash('f');
        let (revoke_pid_tx, revoke_pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let revoke = tokio::spawn(async move {
            let mut tx = owner_pool.begin().await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *tx)
                .await?;
            let _ = revoke_pid_tx.send(pid);
            sqlx::query("SELECT daily_0060_source_lock($1,true)")
                .bind(owner_id)
                .execute(&mut *tx)
                .await?;
            let revoked: bool = sqlx::query_scalar("SELECT revoke_owner_beta_daily_input($1,$2)")
                .bind(admission)
                .bind(evidence)
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok::<_, sqlx::Error>(revoked)
        });
        let revoke_pid =
            match tokio::time::timeout(std::time::Duration::from_secs(3), revoke_pid_rx).await {
                Ok(Ok(pid)) => Some(pid),
                _ => None,
            };
        let blocking = if let Some(pid) = revoke_pid {
            wait_for_pg_blocker(&db, pid, applying_pid)
                .await
                .map_err(|error| error.to_string())
        } else {
            Err("revocation backend PID was not reported".to_owned())
        };
        let advisory_wait = if let Some(pid) = revoke_pid {
            backend_waits_on_advisory_lock(&db, pid).await
        } else {
            Ok(false)
        };
        let apply_commit = applying.commit().await;
        let mut revoke = revoke;
        let revoke_result =
            match tokio::time::timeout(std::time::Duration::from_secs(8), &mut revoke).await {
                Ok(result) => Some(result),
                Err(_) => {
                    revoke.abort();
                    let _ = revoke.await;
                    None
                }
            };
        apply_commit?;
        let revoked = revoke_result
            .ok_or("exclusive revocation did not finish after apply commit")?
            .map_err(|e| std::io::Error::other(e.to_string()))??;
        if let Err(error) = blocking {
            return Err(error.into());
        }
        assert!(
            advisory_wait?,
            "revocation must wait on the shared source advisory lock"
        );
        assert!(
            revoked,
            "the exclusive source-lock transaction commits the real revocation"
        );
        let replay: Value =
            sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
                .bind(preview)
                .bind(packet["preview_token"].as_str().ok_or("manual token")?)
                .bind(&f.digest)
                .fetch_one(&db.app)
                .await?;
        assert_eq!(
            replay["outcome"], "REPLAY",
            "committed replay identity survives later admission revocation"
        );
        assert_eq!(
            replay["data"]["pending_target_id"],
            applied["data"]["pending_target_id"]
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM pending_targets WHERE account_id=$1"
            )
            .bind(f.account)
            .fetch_one(&db.owner)
            .await?,
            1
        );
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_apply_waits_for_committed_exclusive_revocation() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, packet) = preview_packet(&db, &f, "MANUAL", false).await?;
        let mut tx = db.worker.begin().await?;
        assert_eq!(
            publish_in(&mut tx, &f, preview, job, &packet).await?["outcome"],
            "OK"
        );
        finish_preview(&mut tx, job).await?;
        tx.commit().await?;

        let control_before: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_before: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;

        // The exclusive revocation remains uncommitted while the serving call
        // waits; its advisory lock and row changes commit together.
        let mut revocation = db.owner.begin().await?;
        let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *revocation)
            .await?;
        let revoked: bool = sqlx::query_scalar(
            "SELECT revoke_owner_beta_daily_input($1,$2)",
        )
        .bind(&f.witness.admission)
        .bind(hash('f'))
        .fetch_one(&mut *revocation)
        .await?;
        assert!(revoked);
        let exclusive_lock_held: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND mode='ExclusiveLock' AND granted)",
        )
        .bind(revoke_pid)
        .fetch_one(&db.owner)
        .await?;

        let app_pool = db.app.clone();
        let digest = f.digest.clone();
        let token = packet["preview_token"].as_str().ok_or("manual token")?.to_owned();
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let waiter = tokio::spawn(async move {
            let mut connection = app_pool.acquire().await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            let _ = pid_tx.send(pid);
            let result = sqlx::query_scalar::<_, Value>(
                "SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)",
            )
            .bind(preview)
            .bind(token)
            .bind(digest)
            .fetch_one(&mut *connection)
            .await;
            Ok::<_, sqlx::Error>(result)
        });
        let waiter_pid = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            pid_rx,
        )
        .await
        {
            Ok(Ok(pid)) => Some(pid),
            _ => None,
        };
        let blocked = if let Some(pid) = waiter_pid {
            wait_for_pg_blocker(&db, pid, revoke_pid)
                .await
                .map_err(|error| error.to_string())
        } else {
            Err("apply waiter backend PID was not reported".to_owned())
        };
        let commit = revocation.commit().await;
        let mut waiter = waiter;
        let waiter_result = match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            &mut waiter,
        )
        .await
        {
            Ok(result) => Some(result),
            Err(_) => {
                waiter.abort();
                let _ = waiter.await;
                None
            }
        };
        commit?;
        if let Err(error) = blocked {
            return Err(error.into());
        }
        assert!(exclusive_lock_held, "the blocking transaction owns the exclusive source lock");
        let call = waiter_result
            .ok_or("apply did not finish after the revocation commit")?
            .map_err(|e| std::io::Error::other(e.to_string()))??;
        denied(call, "42501");
        let control_after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(c) FROM owner_beta_paper_control c WHERE id='etf11'",
        )
        .fetch_one(&db.owner)
        .await?;
        let counts_after: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(control_after, control_before);
        assert_eq!(counts_after, counts_before);
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_auto_first_manual_reuses_exact_lineage_and_single_target() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let (auto,auto_job,auto_packet)=preview_packet(&db,&f,"AUTO",false).await?;
        let manual:Value=sqlx::query_scalar("SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'manual-request',$3)")
            .bind(f.account).bind(f.witness.run).bind(&f.digest).fetch_one(&db.app).await?;
        assert_eq!(manual["outcome"],"OK");
        assert_eq!(manual["data"]["lineage_id"],auto_packet["lineage_id"]);
        let (manual,manual_job,manual_packet)=preview_packet(&db,&f,"MANUAL",false).await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,auto,auto_job,&auto_packet).await?["outcome"],"OK");
        finish_preview(&mut tx,auto_job).await?;
        tx.commit().await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,manual,manual_job,&manual_packet).await?["outcome"],"OK");
        finish_preview(&mut tx,manual_job).await?;
        tx.commit().await?;
        let conflict:Value=sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
            .bind(manual).bind(manual_packet["preview_token"].as_str().ok_or("manual token")?)
            .bind(&f.digest).fetch_one(&db.app).await?;
        assert_eq!(conflict["outcome"],"CONFLICT");
        let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM owner_beta_paper_lineage WHERE account_id=$1 AND recommendation_run_id=$2),(SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)")
            .bind(f.account).bind(f.witness.run).fetch_one(&db.owner).await?;
        assert_eq!(counts,(1,1,1));
        let modes:(String,String)=sqlx::query_as("SELECT (SELECT status FROM paper_rebalance_previews WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)")
            .bind(auto).bind(manual).fetch_one(&db.owner).await?;
        assert_eq!(modes,("APPLIED".into(),"READY".into()));
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_manual_first_auto_reuses_exact_lineage_and_single_target() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let (manual,manual_job,manual_packet)=preview_packet(&db,&f,"MANUAL",false).await?;
        let automatic:Value=sqlx::query_scalar("SELECT queue_owner_beta_paper_target_v1($1,1,$2)")
            .bind(f.witness.run).bind(&f.digest).fetch_one(&db.worker).await?;
        assert_eq!(automatic["outcome"],"QUEUED");
        assert_eq!(automatic["data"]["lineage_id"],manual_packet["lineage_id"]);
        let (auto,auto_job,auto_packet)=preview_packet(&db,&f,"AUTO",false).await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,manual,manual_job,&manual_packet).await?["outcome"],"OK");
        finish_preview(&mut tx,manual_job).await?;
        tx.commit().await?;
        let applied:Value=sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
            .bind(manual).bind(manual_packet["preview_token"].as_str().ok_or("manual token")?)
            .bind(&f.digest).fetch_one(&db.app).await?;
        assert_eq!(applied["outcome"],"OK");
        let mut tx=db.worker.begin().await?;
        let auto_result=publish_in(&mut tx,&f,auto,auto_job,&auto_packet).await?;
        assert_eq!(auto_result["outcome"],"CONFLICT");
        finish_preview(&mut tx,auto_job).await?;
        tx.commit().await?;
        let ready_status:String=sqlx::query_scalar("SELECT status FROM paper_rebalance_previews WHERE id=$1").bind(auto).fetch_one(&db.owner).await?;
        assert_eq!(ready_status,"READY","AUTO conflict remains a persisted retryable READY preview");
        let before_retry:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)")
            .bind(f.account).fetch_one(&db.owner).await?;
        let retry:Value=sqlx::query_scalar("SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)")
            .bind(auto).bind(&f.digest).fetch_one(&db.worker).await?;
        assert_eq!(retry["outcome"],"CONFLICT","persisted AUTO retry must refuse the MANUAL-owned pending target");
        let after_retry:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)")
            .bind(f.account).fetch_one(&db.owner).await?;
        assert_eq!(after_retry,before_retry,"retry refusal cannot duplicate target/session economics");
        let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM owner_beta_paper_lineage WHERE account_id=$1 AND recommendation_run_id=$2),(SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)")
            .bind(f.account).bind(f.witness.run).fetch_one(&db.owner).await?;
        assert_eq!(counts,(1,1,1));
        let modes:(String,String)=sqlx::query_as("SELECT (SELECT status FROM paper_rebalance_previews WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)")
            .bind(manual).bind(auto).fetch_one(&db.owner).await?;
        assert_eq!(modes,("APPLIED".into(),"READY".into()));
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_lineage_conflicting_calendar_pin_fails_closed() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let automatic: Value = sqlx::query_scalar(
            "SELECT queue_owner_beta_paper_target_v1($1,1,$2)",
        )
        .bind(f.witness.run)
        .bind(&f.digest)
        .fetch_one(&db.worker)
        .await?;
        assert_eq!(automatic["outcome"], "QUEUED");
        let lineage = Uuid::parse_str(
            automatic["data"]["lineage_id"]
                .as_str()
                .ok_or("automatic lineage")?,
        )?;
        let (prior_calendar, prior_effective): (String, String) = sqlx::query_as(
            "SELECT calendar_evidence_sha256,effective_session::text FROM owner_beta_paper_lineage WHERE id=$1",
        )
        .bind(lineage)
        .fetch_one(&db.owner)
        .await?;
        let dates: Vec<String> = sqlx::query_scalar(
            "SELECT ((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+n)::text FROM generate_series(-2,2) n ORDER BY n",
        )
        .fetch_all(&db.owner)
        .await?;
        let days = Value::Array(
            dates
                .iter()
                .enumerate()
                .map(|(i, date)| {
                    json!({"date":date,"kind":"TRADING","ordinal":i,"row_hash":hash('4')})
                })
                .collect(),
        );
        let mut connection = db.owner.acquire().await?;
        calendar_claim(
            &mut connection,
            db.owner_id,
            &hash('0'),
            &hash('b'),
            &days,
            false,
        )
        .await?;
        let (changed_calendar, changed_effective): (String, String) = sqlx::query_as(
            "SELECT calendar_evidence_sha256,effective_session::text FROM paper_0067_next_session($1,$2,$3::text::date)",
        )
        .bind(db.owner_id)
        .bind(hash('b'))
        .bind(&f.witness.session)
        .fetch_one(&mut *connection)
        .await?;
        assert_eq!(changed_effective, prior_effective, "fixture changes the evidence pin for the same natural lineage identity");
        assert_ne!(changed_calendar, prior_calendar, "fixture must select a changed immutable calendar pin");
        denied(
            sqlx::query("SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'manual-request',$3)")
                .bind(f.account)
                .bind(f.witness.run)
                .bind(&f.digest)
                .fetch_one(&db.app)
                .await,
            "23505",
        );
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM owner_beta_paper_lineage WHERE account_id=$1 AND recommendation_run_id=$2),(SELECT count(*) FROM paper_rebalance_previews WHERE account_id=$1)",
        )
        .bind(f.account)
        .bind(f.witness.run)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(counts, (1, 1), "conflicting immutable evidence must not rewrite lineage or create MANUAL preview");
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_canceled_job_projects_failed_owner_preview_only() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let queued:Value=sqlx::query_scalar("SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'cancel-preview',$3)")
            .bind(f.account).bind(f.witness.run).bind(&f.digest).fetch_one(&db.app).await?;
        assert_eq!(queued["outcome"],"OK");
        let preview=Uuid::parse_str(queued["data"]["preview_id"].as_str().ok_or("preview id")?)?;
        let job=Uuid::parse_str(queued["data"]["job_id"].as_str().ok_or("job id")?)?;
        let foreign=Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'https://sql.invalid','terminal-foreign','terminal-foreign@invalid')")
            .bind(foreign).execute(&db.owner).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id) VALUES($1,'owner')").bind(foreign).execute(&db.owner).await?;
        // Exercise terminal projection through the existing engine role while preserving a foreign actor.
        let mut tx=db.worker.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(foreign.to_string()).execute(&mut *tx).await?;
        let changed=sqlx::query("UPDATE jobs SET status='CANCELED',error_code='canceled',finished_at=clock_timestamp() WHERE id=$1 AND owner_user_id=$2 AND job_type='owner_beta_paper_preview' AND status='QUEUED'")
            .bind(job).bind(db.owner_id).execute(&mut *tx).await?.rows_affected();
        assert_eq!(changed,1,"the real queued cancellation must occur before checking its projection");
        let actor_after:String=sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *tx).await?;
        assert_eq!(actor_after,foreign.to_string(),"terminal projection must preserve the caller actor");
        tx.commit().await?;
        let projected:(String,Value,bool)=sqlx::query_as("SELECT status,error_json,completed_at IS NOT NULL AND completed_at<=clock_timestamp() FROM paper_rebalance_previews WHERE id=$1 AND owner_user_id=$2 AND job_id=$3")
            .bind(preview).bind(db.owner_id).bind(job).fetch_one(&db.app).await?;
        assert_eq!(projected.0,"FAILED");
        assert_eq!(projected.1["code"],"OWNER_BETA_PAPER_PREVIEW_CANCELED");
        assert!(projected.2,"projection completion time must come from the database clock");
        let other=super::daily_0060_login(&db.url,&db.name,"app",foreign).await?;
        let visible:i64=sqlx::query_scalar("SELECT count(*) FROM paper_rebalance_previews WHERE id=$1")
            .bind(preview).fetch_one(&other).await?;
        assert_eq!(visible,0,"another Owner cannot observe the projected preview");
        other.close().await;
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_terminal_projection_ignores_owner_job_and_payload_mismatches() -> Daily0060Result
{
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let queued: Value = sqlx::query_scalar(
            "SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'projection-mismatch',$3)",
        )
        .bind(f.account)
        .bind(f.witness.run)
        .bind(&f.digest)
        .fetch_one(&db.app)
        .await?;
        assert_eq!(queued["outcome"], "OK");
        let preview = Uuid::parse_str(queued["data"]["preview_id"].as_str().ok_or("preview")?)?;
        let job = Uuid::parse_str(queued["data"]["job_id"].as_str().ok_or("job")?)?;
        let foreign = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'https://sql.invalid','projection-foreign','projection-foreign@invalid')")
            .bind(foreign)
            .execute(&db.owner)
            .await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id) VALUES($1,'owner')")
            .bind(foreign)
            .execute(&db.owner)
            .await?;

        let wrong_job = Uuid::new_v4();
        let wrong_payload = Uuid::new_v4();
        let foreign_job = Uuid::new_v4();
        let valid_payload = json!({
            "schema":"owner-beta-paper-preview-job-v1",
            "preview_id":preview.to_string(),
            "input_kind":"OWNER_BETA_DAILY_V1",
            "apply_mode":"MANUAL"
        });
        let wrong_payload_json = json!({
            "schema":"owner-beta-paper-preview-job-v1",
            "preview_id":preview.to_string(),
            "input_kind":"OWNER_BETA_DAILY_V0",
            "apply_mode":"MANUAL"
        });
        for (fake_job, owner, payload) in [
            (wrong_job, db.owner_id, valid_payload.clone()),
            (wrong_payload, db.owner_id, wrong_payload_json),
            (foreign_job, foreign, valid_payload),
        ] {
            // The migration owner is subject to FORCE RLS; seed each row with its own actor.
            let mut seed=db.owner.begin().await?;
            sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
                .bind(owner.to_string()).execute(&mut *seed).await?;
            sqlx::query("INSERT INTO jobs(id,owner_user_id,job_type,status,payload_json) VALUES($1,$2,'owner_beta_paper_preview','QUEUED',$3)")
                .bind(fake_job)
                .bind(owner)
                .bind(payload)
                .execute(&mut *seed)
                .await?;
            seed.commit().await?;
        }

        for (fake_job, actor) in [
            (wrong_job, None),
            (wrong_payload, Some(String::new())),
            (foreign_job, Some(foreign.to_string())),
        ] {
            let mut tx = db.worker.begin().await?;
            if let Some(actor) = actor.as_deref() {
                sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
                    .bind(actor)
                    .execute(&mut *tx)
                    .await?;
            }
            let actor_before: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *tx)
                    .await?;
            let changed = sqlx::query("UPDATE jobs SET status='CANCELED',error_code='canceled',finished_at=clock_timestamp() WHERE id=$1 AND status='QUEUED'")
                .bind(fake_job)
                .execute(&mut *tx)
                .await?;
            assert_eq!(changed.rows_affected(), 1);
            let actor_after: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *tx)
                    .await?;
            assert_same_actor(&actor_before, &actor_after);
            tx.commit().await?;
            let untouched: (String, Option<Value>, Option<String>) = sqlx::query_as(
                "SELECT status,error_json,completed_at::text FROM paper_rebalance_previews WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
            )
            .bind(preview)
            .bind(db.owner_id)
            .bind(job)
            .fetch_one(&db.owner)
            .await?;
            assert_eq!(untouched, ("PENDING".into(), None, None));
        }

        let mut tx = db.worker.begin().await?;
        let actor_before: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        let changed = sqlx::query("UPDATE jobs SET status='CANCELED',error_code='canceled',finished_at=clock_timestamp() WHERE id=$1 AND owner_user_id=$2 AND status='QUEUED'")
            .bind(job)
            .bind(db.owner_id)
            .execute(&mut *tx)
            .await?;
        assert_eq!(changed.rows_affected(), 1);
        let actor_after: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        assert_same_actor(&actor_before, &actor_after);
        tx.commit().await?;
        let projected: (String, Value) = sqlx::query_as(
            "SELECT status,error_json FROM paper_rebalance_previews WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
        )
        .bind(preview)
        .bind(db.owner_id)
        .bind(job)
        .fetch_one(&db.app)
        .await?;
        assert_eq!(projected.0, "FAILED");
        assert_eq!(projected.1["code"], "OWNER_BETA_PAPER_PREVIEW_CANCELED");
        let other = super::daily_0060_login(&db.url, &db.name, "app", foreign).await?;
        let visible: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM paper_rebalance_previews WHERE id=$1",
        )
        .bind(preview)
        .fetch_one(&other)
        .await?;
        assert_eq!(visible, 0);
        other.close().await;
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_attempts_exhausted_projects_running_preview_and_preserves_actor()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, _) = preview_packet(&db, &f, "MANUAL", false).await?;
        let mut tx = db.worker.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind("")
            .execute(&mut *tx)
            .await?;
        let actor_before: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        let attempt = sqlx::query("UPDATE job_attempts SET outcome='FAILED',finished_at=clock_timestamp() WHERE job_id=$1 AND outcome='RUNNING'")
            .bind(job)
            .execute(&mut *tx)
            .await?;
        assert_eq!(attempt.rows_affected(), 1);
        let terminal = sqlx::query("UPDATE jobs SET status='FAILED',error_code='attempts_exhausted',locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp() WHERE id=$1 AND owner_user_id=$2 AND job_type='owner_beta_paper_preview' AND status='RUNNING'")
            .bind(job)
            .bind(db.owner_id)
            .execute(&mut *tx)
            .await?;
        assert_eq!(terminal.rows_affected(), 1);
        let actor_after: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        assert_same_actor(&actor_before, &actor_after);
        tx.commit().await?;
        let projected: (String, Value, bool) = sqlx::query_as(
            "SELECT status,error_json,completed_at IS NOT NULL AND completed_at<=clock_timestamp() FROM paper_rebalance_previews WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
        )
        .bind(preview)
        .bind(db.owner_id)
        .bind(job)
        .fetch_one(&db.app)
        .await?;
        assert_eq!(projected.0, "FAILED");
        assert_eq!(projected.1["code"], "OWNER_BETA_PAPER_PREVIEW_ATTEMPTS_EXHAUSTED");
        assert!(projected.2);
        let economics: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(economics, (0, 0));
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_existing_failed_preview_survives_later_terminal_job_projection()
-> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let (preview, job, packet) = preview_packet(&db, &f, "MANUAL", false).await?;
        let error = json!({
            "code":"SYNTHETIC_FAILURE",
            "message":"already failed preview fixture",
            "attempt_id":packet["attempt_id"],
            "attempt_no":packet["attempt_no"],
            "worker_id":packet["worker_id"],
            "process_config_sha256":f.digest
        });
        let failed: Value = sqlx::query_scalar(
            "SELECT fail_owner_beta_paper_preview_v1($1,$2,$3,$4)",
        )
        .bind(preview)
        .bind(job)
        .bind(error)
        .bind(&f.digest)
        .fetch_one(&db.worker)
        .await?;
        assert_eq!(failed["outcome"], "OK");
        let before: Value = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
        )
        .bind(preview)
        .bind(db.owner_id)
        .bind(job)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(before["status"], "FAILED");
        assert_eq!(before["error_json"]["code"], "SYNTHETIC_FAILURE");

        let mut tx = db.worker.begin().await?;
        let actor = Uuid::new_v4().to_string();
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
            .bind(&actor)
            .execute(&mut *tx)
            .await?;
        let actor_before: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        let attempt = sqlx::query("UPDATE job_attempts SET outcome='FAILED',finished_at=clock_timestamp() WHERE job_id=$1 AND outcome='RUNNING'")
            .bind(job)
            .execute(&mut *tx)
            .await?;
        assert_eq!(attempt.rows_affected(), 1);
        sqlx::query("UPDATE jobs SET status='FAILED',error_code='attempts_exhausted',locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp() WHERE id=$1 AND status='RUNNING'")
            .bind(job)
            .execute(&mut *tx)
            .await?;
        let actor_after: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *tx)
                .await?;
        assert_same_actor(&actor_before, &actor_after);
        tx.commit().await?;
        let after: Value = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE id=$1 AND owner_user_id=$2 AND job_id=$3",
        )
        .bind(preview)
        .bind(db.owner_id)
        .bind(job)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(after, before, "an existing FAILED error/result/completion is immutable");
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_terminal_job_keeps_ready_and_applied_previews_unchanged() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=enabled_paper(&db,false).await?;
        let (manual,manual_job,manual_packet)=preview_packet(&db,&f,"MANUAL",false).await?;
        let (auto,auto_job,auto_packet)=preview_packet(&db,&f,"AUTO",false).await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,manual,manual_job,&manual_packet).await?["outcome"],"OK");
        tx.commit().await?;
        let mut tx=db.worker.begin().await?;
        assert_eq!(publish_in(&mut tx,&f,auto,auto_job,&auto_packet).await?["outcome"],"OK");
        tx.commit().await?;
        let before:Value=sqlx::query_scalar("SELECT jsonb_build_array((SELECT jsonb_build_object('status',status,'error_json',error_json,'result_json',result_json,'completed_at',completed_at,'applied_at',applied_at,'pending_target_id',pending_target_id) FROM paper_rebalance_previews WHERE id=$1),(SELECT jsonb_build_object('status',status,'error_json',error_json,'result_json',result_json,'completed_at',completed_at,'applied_at',applied_at,'pending_target_id',pending_target_id) FROM paper_rebalance_previews WHERE id=$2))")
            .bind(manual).bind(auto).fetch_one(&db.owner).await?;
        let mut tx=db.worker.begin().await?;
        for job in [manual_job,auto_job] {
            sqlx::query("UPDATE job_attempts SET outcome='FAILED',finished_at=clock_timestamp() WHERE job_id=$1 AND outcome='RUNNING'")
                .bind(job).execute(&mut *tx).await?;
            sqlx::query("UPDATE jobs SET status='FAILED',error_code='worker_failed',locked_by=NULL,locked_at=NULL,finished_at=clock_timestamp() WHERE id=$1 AND job_type='owner_beta_paper_preview' AND status='RUNNING'")
                .bind(job).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        let after:Value=sqlx::query_scalar("SELECT jsonb_build_array((SELECT jsonb_build_object('status',status,'error_json',error_json,'result_json',result_json,'completed_at',completed_at,'applied_at',applied_at,'pending_target_id',pending_target_id) FROM paper_rebalance_previews WHERE id=$1),(SELECT jsonb_build_object('status',status,'error_json',error_json,'result_json',result_json,'completed_at',completed_at,'applied_at',applied_at,'pending_target_id',pending_target_id) FROM paper_rebalance_previews WHERE id=$2))")
            .bind(manual).bind(auto).fetch_one(&db.owner).await?;
        assert_eq!(before[0]["status"],"READY");
        assert_eq!(before[1]["status"],"APPLIED");
        assert_eq!(after,before,"terminal jobs leave READY/APPLIED results and completion identity byte-for-byte unchanged");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL")
            .bind(f.account).fetch_one(&db.owner).await?,1,"terminal projection cannot create or duplicate economics");
        Ok(())
    }.await;
    db.close().await?;
    clock.set(0).await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_snapshot_locks_job_before_preview_terminal_projection() -> Daily0060Result {
    terminal_serving_lock_order_case("snapshot", None, None).await
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_publish_locks_job_before_preview_terminal_projection() -> Daily0060Result {
    terminal_serving_lock_order_case(
        "publish",
        Some(String::new()),
        Some(Uuid::new_v4().to_string()),
    )
    .await
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_fail_locks_job_before_preview_terminal_projection() -> Daily0060Result {
    terminal_serving_lock_order_case(
        "fail",
        Some(Uuid::new_v4().to_string()),
        Some(String::new()),
    )
    .await
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn paper_0067_terminal_projection_failure_rolls_back_job_transition() -> Daily0060Result {
    let db = isolated_db().await?;
    let clock = FixtureClock::new().await?;
    clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = enabled_paper(&db, false).await?;
        let queued: Value = sqlx::query_scalar(
            "SELECT submit_owner_beta_paper_preview_v1($1,$2,1,'projection-rollback',$3)",
        )
        .bind(f.account)
        .bind(f.witness.run)
        .bind(&f.digest)
        .fetch_one(&db.app)
        .await?;
        assert_eq!(queued["outcome"], "OK");
        let preview = Uuid::parse_str(queued["data"]["preview_id"].as_str().ok_or("preview")?)?;
        let job = Uuid::parse_str(queued["data"]["job_id"].as_str().ok_or("job")?)?;

        let mut preview_lock = db.owner.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *preview_lock)
            .await?;
        let _: Uuid = sqlx::query_scalar(
            "SELECT id FROM paper_rebalance_previews WHERE id=$1 FOR UPDATE",
        )
        .bind(preview)
        .fetch_one(&mut *preview_lock)
        .await?;
        let worker_pool = db.worker.clone();
        let owner_id = db.owner_id;
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel::<i32>();
        let terminal = tokio::spawn(async move {
            let mut connection = worker_pool.acquire().await?;
            sqlx::query("SELECT set_config('app.actor_user_id',$1,false)")
                .bind("")
                .execute(&mut *connection)
                .await?;
            let actor_before: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *connection)
                    .await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *connection)
                .await?;
            let _ = pid_tx.send(pid);
            let update = sqlx::query("UPDATE jobs SET status='CANCELED',error_code='canceled',finished_at=clock_timestamp() WHERE id=$1 AND owner_user_id=$2 AND job_type='owner_beta_paper_preview' AND status='QUEUED'")
                .bind(job)
                .bind(owner_id)
                .execute(&mut *connection)
                .await
                .map(|result| result.rows_affected());
            let actor_after: Option<String> =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                    .fetch_one(&mut *connection)
                    .await?;
            sqlx::query("RESET app.actor_user_id")
                .execute(&mut *connection)
                .await?;
            Ok::<_, sqlx::Error>((update, actor_before, actor_after))
        });
        let terminal_pid = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            pid_rx,
        )
        .await
        {
            Ok(Ok(pid)) => Some(pid),
            _ => None,
        };
        let blocked = if let Some(pid) = terminal_pid {
            wait_for_pg_blocker(&db, pid, blocker_pid)
                .await
                .map_err(|error| error.to_string())
        } else {
            Err("terminal backend PID was not reported".to_owned())
        };
        let mut terminal = terminal;
        let terminal_result = tokio::time::timeout(
            std::time::Duration::from_secs(7),
            &mut terminal,
        )
        .await
        .ok();
        let completed_before_unlock = terminal_result.is_some();
        let preview_unlock = preview_lock.rollback().await;
        let terminal_result = match terminal_result {
            Some(result) => Some(result),
            None => match tokio::time::timeout(
                std::time::Duration::from_secs(4),
                &mut terminal,
            )
            .await
            {
                Ok(result) => Some(result),
                Err(_) => {
                    terminal.abort();
                    let _ = terminal.await;
                    None
                }
            },
        };
        if let Err(error) = blocked {
            return Err(error.into());
        }
        preview_unlock?;
        let terminal_result = terminal_result
            .ok_or("terminal update task did not finish")?
            .map_err(|error| std::io::Error::other(error.to_string()))??;
        assert_same_actor(&terminal_result.1, &terminal_result.2);
        match &terminal_result.0 {
            Ok(_rows) => emit_r5_paper_test_witness(
                "TERMINAL_UPDATE",
                "UPDATE_OK",
                None,
                None,
                None,
                None,
                Some(completed_before_unlock),
            ),
            Err(error) => {
                if let Some(database_error) = error.as_database_error() {
                    if let Some(code) = database_error.code() {
                        emit_r5_paper_test_witness(
                            "TERMINAL_UPDATE",
                            "DATABASE_ERROR",
                            Some(code.as_ref()),
                            None,
                            None,
                            None,
                            Some(completed_before_unlock),
                        );
                    } else {
                        emit_r5_paper_test_witness(
                            "TERMINAL_UPDATE",
                            "DATABASE_ERROR",
                            None,
                            None,
                            None,
                            None,
                            Some(completed_before_unlock),
                        );
                    }
                } else {
                    emit_r5_paper_test_witness(
                        "TERMINAL_UPDATE",
                        "NON_DATABASE_ERROR",
                        None,
                        None,
                        None,
                        None,
                        Some(completed_before_unlock),
                    );
                }
            }
        }
        assert_eq!(
            terminal_result
                .0
                .as_ref()
                .err()
                .and_then(sqlx::Error::as_database_error)
                .and_then(|error| error.code())
                .as_deref(),
            Some("55P03"),
            "real projection lock timeout must reject the enclosing job transition"
        );
        let states: (String, String) = sqlx::query_as(
            "SELECT (SELECT status FROM jobs WHERE id=$1),(SELECT status FROM paper_rebalance_previews WHERE id=$2)",
        )
        .bind(job)
        .bind(preview)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(states, ("QUEUED".into(), "PENDING".into()));
        let economics: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE account_id=$1 AND beta_lineage_id IS NOT NULL),(SELECT count(*) FROM owner_beta_paper_sessions WHERE account_id=$1)",
        )
        .bind(f.account)
        .fetch_one(&db.owner)
        .await?;
        assert_eq!(economics, (0, 0));
        Ok(())
    }
    .await;
    db.close().await?;
    clock.set(0).await?;
    result
}

// Relational SQL fixture, not native Source/Raw evidence. All old identities are
// read from actual rows; the existing synthetic predecessor helper is explicit.
struct HistoricalRecoveryFixture {
    paper: SqlPaper,
    decision: Value,
    signal_readback: Value,
    execution_readback: Value,
    current_cohort: Value,
}

async fn historical_recovery_fixture(
    db: &Daily0060Db, clock: &FixtureClock, initial: i64, settled: bool,
) -> Daily0060Result<HistoricalRecoveryFixture> {
    let paper = enabled_paper(db, false).await?;
    let (target, _) = auto_target(db, &paper, false).await?;
    clock.set(initial + 86400).await?;
    let day: String = sqlx::query_scalar(
        "SELECT effective_session::text FROM owner_beta_paper_sessions WHERE pending_target_id=$1",
    ).bind(target).fetch_one(&db.owner).await?;
    let execution = seed_witness(db, paper.config, &paper.cohort, &day, Some(&paper.witness.admission), false).await?;
    let signal_readback: Value = sqlx::query_scalar(
        "SELECT read_owner_beta_paper_session_evidence($1::text::date,$2)",
    ).bind(&paper.witness.session).bind(&paper.cohort).fetch_one(&db.worker).await?;
    let execution_readback: Value = sqlx::query_scalar(
        "SELECT read_owner_beta_paper_session_evidence($1::text::date,$2)",
    ).bind(&day).bind(&paper.cohort).fetch_one(&db.worker).await?;
    if settled {
        let packet = settlement_packet(db, &paper, target, &execution.admission, false).await?;
        assert_eq!(settle(db, &paper, target, &packet).await?["outcome"], "OK");
    }
    clock.set(initial + 172800).await?;
    let current_cohort = reenable_retained_fixture_with_daily(db, clock, initial, &paper, target, true).await?;
    let (lineage, lineage_epoch, active_epoch, consent, process, original):
        (Uuid, i64, i64, Uuid, String, String) = sqlx::query_as(
        "SELECT l.id,l.control_epoch,c.control_epoch,c.consent_event_id,c.process_binding_sha256,COALESCE(s.settlement_sha256,daily_0060_digest(paper_0068_canonical_json(COALESCE(s.receipt_json,'null'::jsonb)))) FROM pending_targets t JOIN owner_beta_paper_lineage l ON l.id=t.beta_lineage_id JOIN owner_beta_paper_sessions s ON s.pending_target_id=t.id JOIN owner_beta_paper_control c ON c.account_id=t.account_id WHERE t.id=$1",
    ).bind(target).fetch_one(&db.owner).await?;
    let step = if settled { "VALUE_ONLY" } else { "SETTLE_THEN_VALUE" };
    let generation = Uuid::new_v4();
    let decision_hash: String = sqlx::query_scalar(
        "SELECT daily_0060_digest(paper_0068_canonical_json(jsonb_build_object('effective_session',$1::text,'pending_target_id',$2::text,'step',$3::text)))",
    ).bind(&day).bind(target.to_string()).bind(step).fetch_one(&db.owner).await?;
    let expiry: String = sqlx::query_scalar(
        "SELECT to_char((((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1)::timestamp AT TIME ZONE 'Asia/Seoul') AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
    ).fetch_one(&db.owner).await?;
    let decision = json!({"schema":"owner-beta-paper-recovery-v2","installation_id":Uuid::new_v4(),
        "generation":generation,"account_id":paper.account,"request_key":format!("{generation}:recover"),
        "pending_target_id":target,"lineage_id":lineage,"effective_session":day,"lineage_epoch":lineage_epoch,
        "active_epoch":active_epoch,"original_receipt_sha256":original,"consent_event_id":consent,
        "process_config_sha256":process,"step":step,"signal_admission_sha256":paper.witness.admission,
        "execution_admission_sha256":execution.admission,"decision_sha256":decision_hash,
        "reason_code":"EXPLICIT_RETAINED_RECOVERY","expires_at":expiry});
    Ok(HistoricalRecoveryFixture { paper, decision, signal_readback, execution_readback, current_cohort })
}

async fn historical_session_read(
    connection: &mut sqlx::PgConnection, decision: &Value, readback: &Value,
) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT public.paper_0068_read_recovery_session($1,$2::text::date,$3)")
        .bind(decision).bind(readback["slot"]["session_date"].as_str().expect("actual original date"))
        .bind(&readback["slot"]["cohort"]).fetch_one(connection).await
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_historical_reader_preserves_t_e_across_current_daily_epochs() -> Daily0060Result {
    for settled in [false, true] {
        let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
        let clock = FixtureClock::new().await?;
        let initial = clock.after_cutoff().await?;
        let result: Daily0060Result = async {
            let f = historical_recovery_fixture(&db, &clock, initial, settled).await?;
            assert_ne!(f.current_cohort["activation_epoch"], f.paper.cohort["activation_epoch"]);
            assert_ne!(f.current_cohort["config_epoch"], f.paper.cohort["config_epoch"]);
            let historical_before_current_first: bool = sqlx::query_scalar(
                "SELECT first_auto_session>$1::text::date AND first_auto_session<=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date FROM owner_beta_daily_control WHERE id='etf11'",
            ).bind(f.decision["effective_session"].as_str().ok_or("E date")?).fetch_one(&db.owner).await?;
            assert!(historical_before_current_first, "both original T/E precede valid current first-auto-session");
            let original_count: i64 = sqlx::query_scalar("SELECT count(*) FROM owner_beta_paper_recovery_grants")
                .fetch_one(&db.owner).await?;
            assert_eq!(original_count, 0, "first source verification precedes grant creation");
            // Execute the real native original-selection SQL, including the
            // beta_lineage_id join: beta targets have no legacy run column.
            let native_source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
                "/../../../crates/job-queue/src/owner_beta/paper/operations/protected.rs"));
            let native_sql = native_source.split_once("const RECOVERY_ORIGINAL_SQL: &str = r#\"")
                .ok_or("native original SQL declaration")?.1.split_once("\"#;")
                .ok_or("native original SQL terminator")?.0;
            let original: Value = sqlx::query_scalar(native_sql)
                .bind(f.paper.account)
                .bind(Uuid::parse_str(f.decision["pending_target_id"].as_str().ok_or("target")?)?)
                .bind(f.decision["execution_admission_sha256"].as_str().ok_or("E admission")?)
                .fetch_one(&db.owner).await?;
            assert_eq!(original["lineage"]["recommendation_run_id"], f.signal_readback["candidate_run_id"]);
            assert_eq!(original["signal_cohort"], f.paper.cohort);
            assert_eq!(original["current"]["cohort"], f.current_cohort);
            let reader = login_without_actor(&db, "migration_owner", 1).await?;
            let mut read = reader.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED").execute(&mut *read).await?;
            let original_actor: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *read).await?;
            let before = economic_image(&db, f.paper.account).await?;
            let operations = operation_image(&db).await?;
            for original in [&f.signal_readback, &f.execution_readback] {
                assert_eq!(historical_session_read(&mut read, &f.decision, original).await?, *original);
                assert_actor(&mut read, &original_actor).await?;
            }
            let worker = worker_without_actor(&db, 1).await?;
            denied(sqlx::query_scalar::<_, Value>("SELECT read_owner_beta_paper_session_evidence($1::text::date,$2)")
                .bind(&f.paper.witness.session).bind(&f.paper.cohort).fetch_one(&worker).await, "23514");
            // Exact native financial-job query remains worker-only and selects
            // the immutable original run/job rather than a current reservation.
            let run = Uuid::parse_str(f.signal_readback["candidate_run_id"].as_str().ok_or("original run")?)?;
            let job = Uuid::parse_str(f.signal_readback["job_id"].as_str().ok_or("original job")?)?;
            let payload: Value = sqlx::query_scalar(
                "SELECT j.payload_json FROM public.owner_beta_recommendation_runs r JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id WHERE r.id=$1 AND j.id=$2 AND r.owner_user_id=$3 AND r.strategy_config_id=$4 AND r.status='SUCCEEDED' AND j.status='SUCCEEDED' FOR SHARE OF r,j",
            ).bind(run).bind(job).bind(db.owner_id).bind(f.paper.config).fetch_one(&worker).await?;
            assert_eq!(payload["run_id"], json!(run));
            assert_eq!(payload["trigger"]["config_epoch"], f.paper.cohort["config_epoch"]);
            assert_eq!(payload["trigger"]["activation_epoch"], f.paper.cohort["activation_epoch"]);
            assert_ne!(payload["trigger"]["config_epoch"], f.current_cohort["config_epoch"]);
            let wrong: Option<Value> = sqlx::query_scalar(
                "SELECT j.payload_json FROM public.owner_beta_recommendation_runs r JOIN public.jobs j ON j.id=r.job_id AND j.owner_user_id=r.owner_user_id WHERE r.id=$1 AND j.id=$2 AND r.owner_user_id=$3 AND r.strategy_config_id=$4 AND r.status='SUCCEEDED' AND j.status='SUCCEEDED' FOR SHARE OF r,j",
            ).bind(Uuid::new_v4()).bind(job).bind(db.owner_id).bind(f.paper.config).fetch_optional(&worker).await?;
            assert!(wrong.is_none(), "wrong original run never supplies financial input");
            worker.close().await;
            assert_eq!(before, economic_image(&db, f.paper.account).await?, "read cannot mutate ledger/outbox/session");
            assert_eq!(operations, operation_image(&db).await?, "read cannot mint grant or operation receipt");
            let response: Value = sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)")
                .bind(f.decision["active_epoch"].as_i64().ok_or("active epoch")?)
                .bind(&f.decision).bind(f.decision["request_key"].as_str().ok_or("key")?)
                .fetch_one(&db.owner).await?;
            assert_eq!(response["outcome"], "OK", "retained reader locks cannot self-deadlock grant dispatch");
            // Same retained READ COMMITTED transaction must now see and validate
            // the independently committed grant/dispatch, without rewriting wire.
            for original in [&f.signal_readback, &f.execution_readback] {
                assert_eq!(historical_session_read(&mut read, &f.decision, original).await?, *original);
            }
            read.rollback().await?;
            reader.close().await;
            let granted = operation_image(&db).await?;
            let exact = exact_request("TRANSITION", Some(f.paper.account), json!({
                "expected_epoch":f.decision["active_epoch"],"transition":"RECOVERY",
                "evidence_canonical":exact_json(&f.decision),"request_key":f.decision["request_key"]}));
            assert_eq!(exact_read(&db.owner, &exact).await?["status"], "COMMITTED");
            assert_eq!(granted, operation_image(&db).await?, "exact replay/readback performs zero writes");
            let after = economic_image(&db, f.paper.account).await?;
            for key in ["cash", "positions", "orders", "fills", "equity", "outbox"] {
                assert!(before.get(key).is_some() && after.get(key).is_some());
                assert_eq!(before[key], after[key], "authorization must not repeat economic writes: {key}");
            }
            let until_midnight: i64 = sqlx::query_scalar(
                "SELECT ceil(extract(epoch FROM (((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date+1)::timestamp AT TIME ZONE 'Asia/Seoul')-clock_timestamp()))::bigint",
            ).fetch_one(&db.owner).await?;
            let expired_offset = initial + 4 * 86400 + until_midnight;
            assert!((0..=432000).contains(&expired_offset), "existing fixture clock bound remains fixed");
            clock.set(expired_offset).await?;
            denied(historical_session_read(&mut *db.owner.acquire().await?, &f.decision, &f.signal_readback).await, "23514");
            assert_eq!(exact_read(&db.owner, &exact).await?["status"], "COMMITTED",
                "expired grant is descriptive history, never renewed by read");
            assert_eq!(granted, operation_image(&db).await?);
            Ok(())
        }.await;
        clock.set(0).await?;
        db.close().await?;
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_historical_reader_rejects_identity_rights_actor_and_dispatch_changes() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = historical_recovery_fixture(&db, &clock, initial, true).await?;
        let image = operation_image(&db).await?;
        let economic = economic_image(&db, f.paper.account).await?;
        for (field, value, state) in [
            ("account_id", json!(Uuid::new_v4()), "23514"),
            ("pending_target_id", json!(Uuid::new_v4()), "23514"),
            ("lineage_id", json!(Uuid::new_v4()), "23514"),
            ("signal_admission_sha256", json!(hash('0')), "23514"),
            ("execution_admission_sha256", json!(hash('0')), "23514"),
            ("original_receipt_sha256", json!(hash('0')), "23514"),
            ("process_config_sha256", json!(hash('0')), "42501"),
            ("consent_event_id", json!(Uuid::new_v4()), "42501"),
            ("expires_at", json!("2000-01-01T00:00:00Z"), "23514"),
        ] {
            let mut changed = f.decision.clone();
            changed[field] = value;
            denied(historical_session_read(&mut *db.owner.acquire().await?, &changed, &f.signal_readback).await, state);
        }
        let mut changed = f.signal_readback.clone();
        changed["slot"]["cohort"]["owner_user_id"] = json!(Uuid::new_v4());
        denied(historical_session_read(&mut *db.owner.acquire().await?, &f.decision, &changed).await, "23514");
        let mut tx = db.owner.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(Uuid::new_v4().to_string()).execute(&mut *tx).await?;
        let actor: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *tx).await?;
        sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
        let rejected = historical_session_read(&mut tx, &f.decision, &f.signal_readback).await;
        actor_error(&mut tx, rejected, "42501", &actor).await?;
        tx.rollback().await?;
        for principal in ["app", "worker", "admin", "audit_writer", "research_writer", "daily_input_admitter"] {
            let allowed: bool = sqlx::query_scalar("SELECT has_function_privilege($1,'public.paper_0068_read_recovery_session(jsonb,date,jsonb)','EXECUTE')")
                .bind(principal).fetch_one(&db.owner).await?;
            assert!(!allowed, "{principal} must not acquire historical reader authority");
        }
        let mut role = db.owner.begin().await?;
        sqlx::query("SET LOCAL ROLE migration_owner").execute(&mut *role).await?;
        denied(historical_session_read(&mut role, &f.decision, &f.signal_readback).await, "42501");
        role.rollback().await?;
        // Current rights and strategy semantics are genuine mutable rows.
        // Roll back each mutation; original immutable evidence is never patched.
        for mutation in [
            "UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'",
            "UPDATE accounts SET initial_cash=1 WHERE account_type='PAPER'",
            "UPDATE user_strategy_configs SET config_json=jsonb_set(config_json,'{fast_ma}','51'::jsonb)",
        ] {
            let mut tx = db.owner.begin().await?;
            sqlx::raw_sql(mutation).execute(&mut *tx).await?;
            sqlx::query("SAVEPOINT actor_error").execute(&mut *tx).await?;
            assert!(historical_session_read(&mut tx, &f.decision, &f.signal_readback).await.is_err());
            sqlx::query("ROLLBACK TO SAVEPOINT actor_error").execute(&mut *tx).await?;
            tx.rollback().await?;
        }
        assert_eq!(image, operation_image(&db).await?);
        assert_eq!(economic, economic_image(&db, f.paper.account).await?);
        // Actual server cancellation after actor acquisition, at an observed
        // account-table blocker. No sleep/timeout is accepted as cancellation.
        let mut blocker = db.owner.begin().await?;
        sqlx::raw_sql("LOCK TABLE accounts IN ACCESS EXCLUSIVE MODE").execute(&mut *blocker).await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
        let cancel_reader = login_without_actor(&db, "migration_owner", 1).await?;
        let mut connection = cancel_reader.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection).await?;
        let prior_actor: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
            .fetch_one(&mut *connection).await?;
        sqlx::raw_sql("BEGIN; SAVEPOINT actor_error").execute(&mut *connection).await?;
        let cancel_decision = f.decision.clone();
        let cancel_original = f.signal_readback.clone();
        let task = tokio::spawn(async move {
            let canceled = historical_session_read(&mut connection, &cancel_decision, &cancel_original).await;
            denied(canceled, "57014");
            sqlx::raw_sql("ROLLBACK TO SAVEPOINT actor_error").execute(&mut *connection).await?;
            let observed: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *connection).await?;
            if prior_actor.is_none() {
                assert!(observed.as_deref().is_none_or(str::is_empty));
            } else {
                assert_eq!(observed, prior_actor);
            }
            sqlx::raw_sql("ROLLBACK").execute(&mut *connection).await?;
            Ok::<(), sqlx::Error>(())
        });
        let waited = wait_for_pg_blocker(&db, pid, blocker_pid).await;
        let canceled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)").bind(pid).fetch_one(&db.owner).await?;
        blocker.rollback().await?;
        task.await??;
        waited?;
        assert!(canceled);
        cancel_reader.close().await;
        assert_eq!(image, operation_image(&db).await?);
        assert_eq!(economic, economic_image(&db, f.paper.account).await?);
        let response: Value = sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)")
            .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?).bind(&f.decision)
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        assert_eq!(response["outcome"], "OK");
        let committed_image = operation_image(&db).await?;
        // Deliberate corruption of real committed rows inside rolled-back
        // disposable fixture transactions. No fabricated origin or success.
        for (table, corrupt) in [
            ("owner_beta_paper_recovery_grants", "DELETE FROM owner_beta_paper_recovery_grants WHERE request_key=$1"),
            ("owner_beta_paper_events", "DELETE FROM owner_beta_paper_events WHERE request_key=$1"),
            ("owner_beta_paper_recovery_grants", "UPDATE owner_beta_paper_recovery_grants SET step='SETTLE_THEN_VALUE' WHERE request_key=$1"),
        ] {
            let mut corrupt_tx = db.owner.begin().await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE public.{table} DISABLE TRIGGER USER")))
                .execute(&mut *corrupt_tx).await?;
            sqlx::query(corrupt).bind(f.decision["request_key"].as_str().ok_or("key")?)
                .execute(&mut *corrupt_tx).await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE public.{table} ENABLE TRIGGER USER")))
                .execute(&mut *corrupt_tx).await?;
            sqlx::query("SAVEPOINT reader_denial").execute(&mut *corrupt_tx).await?;
            denied(historical_session_read(&mut corrupt_tx, &f.decision, &f.signal_readback).await, "42501");
            sqlx::query("ROLLBACK TO SAVEPOINT reader_denial").execute(&mut *corrupt_tx).await?;
            corrupt_tx.rollback().await?;
            assert_eq!(committed_image, operation_image(&db).await?, "fault injection must leave original history intact");
        }
        let mut no_dispatch = f.decision.clone();
        let generation = Uuid::new_v4();
        no_dispatch["generation"] = json!(generation);
        no_dispatch["request_key"] = json!(format!("{generation}:recover"));
        denied(historical_session_read(&mut *db.owner.acquire().await?, &no_dispatch, &f.signal_readback).await, "42501");
        sqlx::query("UPDATE owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE request_key=$1")
            .bind(f.decision["request_key"].as_str().ok_or("key")?).execute(&db.owner).await?;
        denied(historical_session_read(&mut *db.owner.acquire().await?, &f.decision, &f.signal_readback).await, "42501");
        sqlx::query("UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'").execute(&db.owner).await?;
        let before_replay = operation_image(&db).await?;
        let exact = exact_request("TRANSITION", Some(f.paper.account), json!({
            "expected_epoch":f.decision["active_epoch"],"transition":"RECOVERY",
            "evidence_canonical":exact_json(&f.decision),"request_key":f.decision["request_key"]}));
        assert_eq!(exact_read(&db.owner, &exact).await?["status"], "COMMITTED", "descriptive replay precedes current rights");
        assert_eq!(before_replay, operation_image(&db).await?);
        Ok(())
    }.await;
    clock.set(0).await?;
    db.close().await?;
    result
}


async fn worker_recovery_read(
    tx: &mut sqlx::PgConnection, f: &HistoricalRecoveryFixture, grant: Uuid,
    action: &str, original: &Value, receipt: Option<&str>,
) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar("SELECT public.paper_0068_read_worker_recovery($1,$2,$3,$4,$5,$6::text::date,$7)")
        .bind(Uuid::parse_str(f.decision["pending_target_id"].as_str().unwrap()).unwrap())
        .bind(grant).bind(f.decision["active_epoch"].as_i64().unwrap())
        .bind(f.decision["process_config_sha256"].as_str().unwrap()).bind(action)
        .bind(original["slot"]["session_date"].as_str().unwrap()).bind(receipt).fetch_one(tx).await
}

async fn recovery_current_snapshot(db: &Daily0060Db, f: &HistoricalRecoveryFixture) -> Daily0060Result<Value> {
    let value: Value = sqlx::query_scalar("SELECT snapshot_owner_beta_paper_target_v1($1,$2,$3)")
        .bind(Uuid::parse_str(f.decision["pending_target_id"].as_str().ok_or("target")?)?)
        .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?)
        .bind(f.decision["process_config_sha256"].as_str().ok_or("process")?)
        .fetch_one(&db.worker).await?;
    assert_eq!(value["outcome"], "OK");
    Ok(value["data"].clone())
}

// Genuine SQL economics/receipts; synthetic relational prices remain explicitly
// separate from the genuine native Source/financial-engine fixture below.
async fn recovery_no_trade_packet(db: &Daily0060Db, f: &HistoricalRecoveryFixture, valuation: bool) -> Daily0060Result<Value> {
    let snapshot = recovery_current_snapshot(db, f).await?;
    let mut packet = if valuation {
        json!({"schema":"owner-beta-paper-valuation-v1","target_id":snapshot["pending_target_id"],
            "control_epoch":snapshot["control_epoch"],"valuation_admission_sha256":f.decision["execution_admission_sha256"],
            "valuation_input_sha256":hash('d'),"settlement_sha256":snapshot["settlement_sha256"],
            "ledger_after_sha256":snapshot["ledger_after_sha256"],"account_state_version":snapshot["account_state_version"],
            "account_state_sha256":snapshot["account_state_sha256"],"closes":[],"equity":"10000000.0000",
            "cash":snapshot["account_state_preimage"]["cash"],"positions_value":"0.0000","currency":"KRW",
            "observed_input":observation(db).await?})
    } else {
        json!({"schema":"owner-beta-paper-settlement-v1","target_id":snapshot["pending_target_id"],
            "lineage_id":snapshot["lineage_id"],"control_epoch":snapshot["control_epoch"],
            "execution_admission_sha256":f.decision["execution_admission_sha256"],"execution_input_sha256":hash('a'),
            "account_state_version":snapshot["account_state_version"],"account_state_sha256":snapshot["initial_account_state_sha256"],
            "ledger_before_sha256":format!("sha256:{}", snapshot["initial_account_state_sha256"].as_str().ok_or("state")?),
            "outcome":"NO_TRADE","reason_code":"SIGNAL_UNCHANGED","observed_input":observation(db).await?,
            "orders":[],"positions_after":[],"cash_after":snapshot["account_state_preimage"]["cash"]})
    };
    let hash: String = sqlx::query_scalar("SELECT daily_0060_digest($1)").bind(exact_json(&packet)).fetch_one(&db.owner).await?;
    packet["calculation_sha256"] = json!(hash);
    Ok(packet)
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_worker_recovery_consumes_both_steps_with_original_receipts() -> Daily0060Result {
    for already_settled in [false, true] {
        let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
        let clock = FixtureClock::new().await?;
        let initial = clock.after_cutoff().await?;
        let result: Daily0060Result = async {
            let f = historical_recovery_fixture(&db, &clock, initial, already_settled).await?;
            let initial_image = economic_image(&db, f.paper.account).await?;
            let response: Value = sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)")
                .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?).bind(&f.decision)
                .bind(f.decision["request_key"].as_str().ok_or("request")?).fetch_one(&db.owner).await?;
            assert_eq!(response["outcome"], "OK");
            let grant: Uuid = sqlx::query_scalar("SELECT grant_id FROM owner_beta_paper_recovery_grants WHERE request_key=$1")
                .bind(f.decision["request_key"].as_str().ok_or("request")?).fetch_one(&db.owner).await?;
            let worker = worker_without_actor(&db, 1).await?;
            let original_settlement = recovery_current_snapshot(&db, &f).await?["settlement_sha256"].clone();
            let mut last_settlement = original_settlement.clone();
            for action in if already_settled { vec!["VALUE"] } else { vec!["SETTLE", "VALUE"] } {
                let packet = recovery_no_trade_packet(&db, &f, action=="VALUE").await?;
                let economic_before = economic_image(&db, f.paper.account).await?;
                let operation_before = operation_image(&db).await?;
                // First exercise real pre-COMMIT rollback, including grant consumption.
                for commit in [false, true] {
                    let mut tx = worker.begin().await?;
                    let actor: Option<String> = sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *tx).await?;
                    for original in [&f.signal_readback, &f.execution_readback] {
                        let read = worker_recovery_read(&mut tx, &f, grant, action, original, None).await?;
                        assert_eq!(read["readback"], *original);
                        assert_eq!(read["selection"]["signal_cohort"], f.paper.cohort);
                        assert_eq!(read["selection"]["decision"], f.decision);
                        assert_actor(&mut tx, &actor).await?;
                    }
                    let query = if action=="SETTLE" { "SELECT commit_owner_beta_paper_settlement_v1($1,$2,$3,$4)" }
                        else { "SELECT commit_owner_beta_paper_valuation_v1($1,$2,$3,$4)" };
                    let reply: Value = sqlx::query_scalar(query)
                        .bind(Uuid::parse_str(f.decision["pending_target_id"].as_str().ok_or("target")?)?)
                        .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?).bind(&packet)
                        .bind(f.decision["process_config_sha256"].as_str().ok_or("process")?).fetch_one(&mut *tx).await?;
                    assert_eq!(reply["outcome"], "OK");
                    for original in [&f.signal_readback, &f.execution_readback] {
                        assert_eq!(worker_recovery_read(&mut tx,&f,grant,action,original,packet["calculation_sha256"].as_str()).await?["readback"],*original);
                        assert_actor(&mut tx,&actor).await?;
                    }
                    if commit { tx.commit().await?; } else {
                        tx.rollback().await?;
                        assert_eq!(economic_before,economic_image(&db,f.paper.account).await?);
                        assert_eq!(operation_before,operation_image(&db).await?);
                    }
                }
                if action=="SETTLE" {
                    last_settlement=packet["calculation_sha256"].clone();
                    assert_eq!(recovery_current_snapshot(&db,&f).await?["settlement_sha256"],last_settlement);
                }
            }
            let (settlement, valuation, grant_settlement, grant_valuation): (String,String,Option<String>,String) = sqlx::query_as(
                "SELECT s.settlement_sha256,s.valuation_sha256,g.settlement_receipt_sha256,g.valuation_receipt_sha256 FROM owner_beta_paper_recovery_grants g JOIN owner_beta_paper_sessions s ON s.pending_target_id=g.pending_target_id WHERE g.grant_id=$1",
            ).bind(grant).fetch_one(&db.owner).await?;
            assert_eq!(json!(settlement),last_settlement);
            assert_eq!(valuation,grant_valuation);
            if already_settled { assert!(grant_settlement.is_none()); assert_eq!(last_settlement,original_settlement); }
            else { assert_eq!(grant_settlement.as_deref(),Some(settlement.as_str())); }
            let final_image=economic_image(&db,f.paper.account).await?;
            for key in ["cash","positions","orders","fills"] { assert_eq!(initial_image[key],final_image[key]); }
            assert_eq!(final_image["outbox"].as_array().ok_or("outbox")?.len(),1);
            assert_eq!(final_image["equity"].as_array().ok_or("equity")?.len(),1);
            denied(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"VALUE",&f.signal_readback,None).await,"42501");
            assert_eq!(final_image,economic_image(&db,f.paper.account).await?,"completed grant cannot authorize a new mutation");
            worker.close().await;
            Ok(())
        }.await;
        clock.set(0).await?; db.close().await?; result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_worker_recovery_rejects_wrong_grant_dispatch_actor_and_current_authority() -> Daily0060Result {
    let db=Daily0060Db::new(require_r5_03_database_url()?,68).await?;
    let clock=FixtureClock::new().await?; let initial=clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=historical_recovery_fixture(&db,&clock,initial,true).await?;
        let worker=worker_without_actor(&db,1).await?;
        denied(worker_recovery_read(&mut *worker.acquire().await?,&f,Uuid::nil(),"VALUE",&f.signal_readback,None).await,"42501");
        let reply:Value=sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)")
            .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?).bind(&f.decision)
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        assert_eq!(reply["outcome"],"OK");
        let grant:Uuid=sqlx::query_scalar("SELECT grant_id FROM owner_beta_paper_recovery_grants WHERE request_key=$1")
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        let economic=economic_image(&db,f.paper.account).await?;
        let operations=operation_image(&db).await?;
        denied(worker_recovery_read(&mut *worker.acquire().await?,&f,Uuid::new_v4(),"VALUE",&f.signal_readback,None).await,"42501");
        denied(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"SETTLE",&f.signal_readback,None).await,"42501");
        denied(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"VALUE",&f.signal_readback,Some(&hash('0'))).await,"42501");
        let mut tx=worker.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(Uuid::new_v4().to_string()).execute(&mut *tx).await?;
        denied(worker_recovery_read(&mut tx,&f,grant,"VALUE",&f.signal_readback,None).await,"42501");tx.rollback().await?;
        let mut tx=worker.begin().await?;
        sqlx::query("SET LOCAL ROLE worker").execute(&mut *tx).await?;
        denied(worker_recovery_read(&mut tx,&f,grant,"VALUE",&f.signal_readback,None).await,"42501");tx.rollback().await?;
        // Remove only the real committed dispatch in this disposable DB, then
        // restore the exact captured row. No manufactured grant or origin.
        let dispatch:Value=sqlx::query_scalar("SELECT to_jsonb(e) FROM owner_beta_paper_events e WHERE request_key=$1")
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        let mut damage=db.owner.begin().await?;
        sqlx::raw_sql("ALTER TABLE owner_beta_paper_events DISABLE TRIGGER USER").execute(&mut *damage).await?;
        sqlx::query("DELETE FROM owner_beta_paper_events WHERE request_key=$1").bind(f.decision["request_key"].as_str().ok_or("key")?).execute(&mut *damage).await?;
        sqlx::raw_sql("ALTER TABLE owner_beta_paper_events ENABLE TRIGGER USER").execute(&mut *damage).await?;
        damage.commit().await?;
        denied(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"VALUE",&f.signal_readback,None).await,"42501");
        let mut restore=db.owner.begin().await?;
        sqlx::raw_sql("ALTER TABLE owner_beta_paper_events DISABLE TRIGGER USER").execute(&mut *restore).await?;
        sqlx::query("INSERT INTO owner_beta_paper_events SELECT (jsonb_populate_record(NULL::owner_beta_paper_events,$1)).*").bind(dispatch).execute(&mut *restore).await?;
        sqlx::raw_sql("ALTER TABLE owner_beta_paper_events ENABLE TRIGGER USER").execute(&mut *restore).await?;
        restore.commit().await?;
        assert_eq!(operations,operation_image(&db).await?);
        // Disposable-row corruption isolates actual rights/economics checks:
        // suppress USER update triggers only within the owner mutation/restore
        // transactions so changing config JSON cannot also increment epochs.
        // Reenable all guards before the real worker sees each committed row.
        for (table,read,change,restore) in [
            ("data_entitlements","SELECT to_jsonb(status) FROM data_entitlements WHERE contract_reference='daily-fixture'",
             "UPDATE data_entitlements SET status='REVOKED' WHERE contract_reference='daily-fixture'",
             "UPDATE data_entitlements SET status=$1#>>'{}' WHERE contract_reference='daily-fixture'"),
            ("accounts","SELECT to_jsonb(initial_cash) FROM accounts WHERE id=$1",
             "UPDATE accounts SET initial_cash=1 WHERE id=$1",
             "UPDATE accounts SET initial_cash=($1#>>'{}')::numeric WHERE id=$2"),
            ("user_strategy_configs","SELECT config_json FROM user_strategy_configs WHERE id=$1",
             "UPDATE user_strategy_configs SET config_json=jsonb_set(config_json,'{fast_ma}','51'::jsonb) WHERE id=$1",
             "UPDATE user_strategy_configs SET config_json=$1 WHERE id=$2")
        ] {
            let saved:Value=if read.contains("accounts") {sqlx::query_scalar(read).bind(f.paper.account).fetch_one(&db.owner).await?}
                else if read.contains("user_strategy_configs") {sqlx::query_scalar(read).bind(f.paper.config).fetch_one(&db.owner).await?}
                else {sqlx::query_scalar(read).fetch_one(&db.owner).await?};
            let id=if read.contains("accounts") {Some(f.paper.account)} else if read.contains("user_strategy_configs") {Some(f.paper.config)} else {None};
            let mut damaged=db.owner.begin().await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE {table} DISABLE TRIGGER USER"))).execute(&mut *damaged).await?;
            let change=sqlx::query(change);let change=if let Some(id)=id {change.bind(id)} else {change};
            change.execute(&mut *damaged).await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE {table} ENABLE TRIGGER USER"))).execute(&mut *damaged).await?;
            damaged.commit().await?;
            assert!(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"VALUE",&f.signal_readback,None).await.is_err());
            let mut restored=db.owner.begin().await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE {table} DISABLE TRIGGER USER"))).execute(&mut *restored).await?;
            let restore=sqlx::query(restore).bind(saved);let restore=if let Some(id)=id {restore.bind(id)} else {restore};
            restore.execute(&mut *restored).await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE {table} ENABLE TRIGGER USER"))).execute(&mut *restored).await?;
            restored.commit().await?;
            assert_eq!(worker_recovery_read(&mut *worker.acquire().await?,&f,grant,"VALUE",&f.signal_readback,None).await?["readback"],f.signal_readback);
        }
        // Wrong current authority remains rejected on the actual worker entry.
        let mut stale=f.decision.clone();stale["active_epoch"]=json!(0);
        let mut changed=HistoricalRecoveryFixture {paper:f.paper,decision:stale,signal_readback:f.signal_readback,execution_readback:f.execution_readback,current_cohort:f.current_cohort};
        assert!(worker_recovery_read(&mut *worker.acquire().await?,&changed,grant,"VALUE",&changed.signal_readback,None).await.is_err());
        assert_eq!(economic,economic_image(&db,changed.paper.account).await?);
        assert_eq!(operations,operation_image(&db).await?);
        changed.decision=f.decision;
        let delta:i64=sqlx::query_scalar("SELECT ceil(extract(epoch FROM expires_at-clock_timestamp()))::bigint+1 FROM owner_beta_paper_recovery_grants WHERE grant_id=$1")
            .bind(grant).fetch_one(&db.owner).await?;
        let expired=initial+4*86400+delta;assert!((0..=432000).contains(&expired));
        clock.set(expired).await?;
        denied(worker_recovery_read(&mut *worker.acquire().await?,&changed,grant,"VALUE",&changed.signal_readback,None).await,"42501");
        assert_eq!(operations,operation_image(&db).await?,"expired read cannot mutate or renew grant");
        clock.set(initial+4*86400).await?;
        // Current epoch and time restored: next refusal proves revocation itself.
        sqlx::query("UPDATE owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE grant_id=$1").bind(grant).execute(&db.owner).await?;
        assert!(worker_recovery_read(&mut *worker.acquire().await?,&changed,grant,"VALUE",&changed.signal_readback,None).await.is_err());
        worker.close().await;
        Ok(())
    }.await;
    clock.set(0).await?;db.close().await?;result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn paper_0068_worker_recovery_holds_fences_and_restores_actor_after_cancel() -> Daily0060Result {
    let db=Daily0060Db::new(require_r5_03_database_url()?,68).await?;
    let clock=FixtureClock::new().await?;let initial=clock.after_cutoff().await?;
    let result:Daily0060Result=async {
        let f=historical_recovery_fixture(&db,&clock,initial,true).await?;
        let reply:Value=sqlx::query_scalar("SELECT transition_owner_beta_paper_v1($1,'RECOVERY',$2,$3)")
            .bind(f.decision["active_epoch"].as_i64().ok_or("epoch")?).bind(&f.decision)
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        assert_eq!(reply["outcome"],"OK");
        let grant:Uuid=sqlx::query_scalar("SELECT grant_id FROM owner_beta_paper_recovery_grants WHERE request_key=$1")
            .bind(f.decision["request_key"].as_str().ok_or("key")?).fetch_one(&db.owner).await?;
        let before=economic_image(&db,f.paper.account).await?;let operations=operation_image(&db).await?;
        let worker=worker_without_actor(&db,1).await?;
        for mutation in [
            "UPDATE owner_beta_paper_recovery_grants SET revoked_at=clock_timestamp() WHERE grant_id=$1",
            "UPDATE owner_beta_paper_control SET updated_at=updated_at WHERE id='etf11'",
            "UPDATE data_entitlements SET status=status WHERE contract_reference='daily-fixture'",
        ] {
            let mut held=worker.begin().await?;
            worker_recovery_read(&mut held,&f,grant,"VALUE",&f.signal_readback,None).await?;
            let blocker:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *held).await?;
            let mut changer=db.owner.acquire().await?;
            let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *changer).await?;
            let task=tokio::spawn(async move {
                let query=sqlx::query(mutation);let query=if mutation.contains("$1") {query.bind(grant)} else {query};
                let changed=query.execute(&mut *changer).await;
                denied(changed,"57014");Ok::<(),sqlx::Error>(())
            });
            let waited=wait_for_pg_blocker(&db,pid,blocker).await;
            let canceled:bool=sqlx::query_scalar("SELECT pg_cancel_backend($1)").bind(pid).fetch_one(&db.owner).await?;
            task.await??;held.rollback().await?;waited?;assert!(canceled);
        }
        // Actual cancellation while the worker reader has acquired its actor.
        let mut blocker=db.owner.begin().await?;
        sqlx::raw_sql("LOCK TABLE accounts IN ACCESS EXCLUSIVE MODE").execute(&mut *blocker).await?;
        let blocker_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
        let mut connection=worker.acquire().await?;
        let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection).await?;
        let prior_actor:Option<String>=sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *connection).await?;
        sqlx::raw_sql("BEGIN; SAVEPOINT actor_error").execute(&mut *connection).await?;
        let original=f.signal_readback.clone();
        let task=tokio::spawn(async move {
            denied(worker_recovery_read(&mut connection,&f,grant,"VALUE",&original,None).await,"57014");
            sqlx::raw_sql("ROLLBACK TO SAVEPOINT actor_error").execute(&mut *connection).await?;
            let restored:Option<String>=sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)").fetch_one(&mut *connection).await?;
            assert_same_actor(&prior_actor,&restored);
            sqlx::raw_sql("ROLLBACK").execute(&mut *connection).await?;Ok::<_,sqlx::Error>(f.paper.account)
        });
        let waited=wait_for_pg_blocker(&db,pid,blocker_pid).await;
        let canceled:bool=sqlx::query_scalar("SELECT pg_cancel_backend($1)").bind(pid).fetch_one(&db.owner).await?;
        let account=task.await??;blocker.rollback().await?;waited?;assert!(canceled);
        worker.close().await;
        assert_eq!(before,economic_image(&db,account).await?);assert_eq!(operations,operation_image(&db).await?);
        Ok(())
    }.await;
    clock.set(0).await?;db.close().await?;result
}
