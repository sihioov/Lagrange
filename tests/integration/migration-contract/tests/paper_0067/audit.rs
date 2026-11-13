//! Source-only until the established isolated hosted PG harness executes these identities.
//! Parent registration: `mod audit;` (coordinator-owned paper_0067.rs).
use super::*;
use sqlx::Row;

const AUDIT_SQL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../deploy/db/owner-beta-paper-audit.sql"
));

async fn read_audit(connection: &mut sqlx::PgConnection) -> Daily0060Result<Vec<Value>> {
    // raw_sql retains each SELECT's one text column across the fixed script.
    let rows = r5_sql_stage(
        "AUDIT_READ",
        sqlx::raw_sql(AUDIT_SQL).fetch_all(connection).await,
    )?;
    rows.iter()
        .map(|r| {
            let text: String = r.try_get("audit_record")?;
            Ok(serde_json::from_str(&text)?)
        })
        .collect()
}

async fn snapshot(db: &Daily0060Db) -> Daily0060Result<Vec<Value>> {
    let mut c = db.owner.acquire().await?;
    sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *c)
        .await?;
    let result = read_audit(&mut c).await;
    sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
    result
}

fn table_rows(rows: &[Value], table: &str) -> Vec<Value> {
    rows.iter()
        .filter(|v| v[0] == "row" && v[2] == table)
        .cloned()
        .collect()
}

async fn seed_account(db: &Daily0060Db, owner: Uuid, id: Uuid) -> Daily0060Result {
    let mut tx = db.owner.begin().await?;
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(owner.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO accounts(id,owner_user_id,account_type,name,currency,status,initial_cash,cost_profile_id,cost_profile_version,created_at,updated_at) VALUES($1,$2,'PAPER','가😀','KRW','ACTIVE',1,'KRX_ETF_DEFAULT',1,'2026-11-06T00:00:00.000001Z','2026-11-06T00:00:00.000001Z')")
        .bind(id).bind(owner).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_read_only_multi_owner_exact_rows_and_actor_restore() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        let other = Uuid::parse_str("22222222-2222-2222-2222-222222222222")?;
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'audit.invalid','other','other@audit.invalid')")
            .bind(other).execute(&db.owner).await?;
        let first = Uuid::parse_str("11111111-1111-1111-1111-111111111111")?;
        let second = Uuid::parse_str("33333333-3333-3333-3333-333333333333")?;
        seed_account(&db, db.owner_id, first).await?;
        seed_account(&db, other, second).await?;
        let mut c = db.owner.acquire().await?;
        sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *c).await?;
        let rows = read_audit(&mut c).await?;
        let actor: String = sqlx::query_scalar("SELECT current_setting('app.actor_user_id')").fetch_one(&mut *c).await?;
        assert_eq!(actor, db.owner_id.to_string());
        let accounts = table_rows(&rows, "accounts");
        assert_eq!(accounts.len(), 2, "both FORCE-RLS owners, exactly once");
        for (row, id, owner) in [(&accounts[0], first, db.owner_id), (&accounts[1], second, other)] {
            assert_eq!(row, &json!(["row","paper_control","accounts",id.to_string(),{
                "id":id.to_string(),"owner_user_id":owner.to_string(),"account_type":"PAPER",
                "name":"가😀","currency":"KRW","status":"ACTIVE","initial_cash":"1.0000",
                "cost_profile_id":"KRX_ETF_DEFAULT","cost_profile_version":1,"paper_state_version":0,
                "created_at":"2026-11-06T00:00:00.000001Z","updated_at":"2026-11-06T00:00:00.000001Z"
            }]));
        }
        // Independent fixture frame: Rust's sorted field serialization, explicit
        // UTF-8 byte lengths, and PG's bytea SHA256 versus a precomputed golden.
        // Neither the producer SQL nor the Python decoder computes this expected value.
        let ordered:std::collections::BTreeMap<_,_>=accounts[1][4].as_object().ok_or("account object")?.iter().collect();
        let encoded=serde_json::to_vec(&ordered)?;
        let key=second.to_string();
        let mut framed=Vec::new();
        for bytes in [b"accounts".as_slice(),key.as_bytes(),encoded.as_slice()] {
            framed.extend_from_slice(bytes.len().to_string().as_bytes());
            framed.push(b':');
            framed.extend_from_slice(bytes);
        }
        let actual:String=sqlx::query_scalar("SELECT encode(sha256($1::bytea),'hex')").bind(framed).fetch_one(&mut *c).await?;
        assert_eq!(actual,"5020c6c048a6ca3be40c58a4156656c7a6c657846be36ad427cf862f82d59436");
        let groups: Vec<_> = rows.iter().filter(|v| v[0] == "group_end").map(|v| v[1].as_str().unwrap()).collect();
        assert_eq!(groups, ["daily","paper_control","paper_jobs","paper_ledger","paper_lineage","paper_operations","paper_outbox","schema_security","simulation"]);
        assert_eq!(rows.last(), Some(&json!(["end"])));
        assert!(rows.iter().any(|r| r[0] == "references_end"));
        let read_only: String = sqlx::query_scalar("SHOW transaction_read_only").fetch_one(&mut *c).await?;
        assert_eq!(read_only,"on");
        sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
        assert_eq!(table_rows(&snapshot(&db).await?,"accounts"), accounts, "audit did not mutate stored rows");
        Ok(())
    }.await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_requires_direct_principal_and_caller_read_only_snapshot() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        for pool in [&db.owner, &db.app] {
            let mut c = pool.acquire().await?;
            sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut *c)
                .await?;
            assert!(read_audit(&mut c).await.is_err());
            sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
        }
        let mut c = db.app.acquire().await?;
        sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *c)
            .await?;
        assert!(read_audit(&mut c).await.is_err());
        sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
        let fresh = login_without_actor(&db, "migration_owner", 1).await?;
        let mut c = fresh.acquire().await?;
        sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *c)
            .await?;
        read_audit(&mut c).await?;
        let actor: Option<String> =
            sqlx::query_scalar("SELECT current_setting('app.actor_user_id',true)")
                .fetch_one(&mut *c)
                .await?;
        assert!(actor.as_deref().is_none_or(str::is_empty));
        sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
        drop(c);
        fresh.close().await;
        Ok(())
    }
    .await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_mutation_deletion_and_repeatable_snapshot() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        let id = Uuid::new_v4();
        seed_account(&db, db.owner_id, id).await?;
        let mut c = db.owner.acquire().await?;
        sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *c)
            .await?;
        let original = table_rows(&read_audit(&mut c).await?, "accounts");
        sqlx::query("UPDATE accounts SET initial_cash=2 WHERE id=$1")
            .bind(id)
            .execute(&db.owner)
            .await?;
        assert_eq!(table_rows(&read_audit(&mut c).await?, "accounts"), original);
        sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
        let changed = table_rows(&snapshot(&db).await?, "accounts");
        assert_ne!(changed, original);
        assert_eq!(changed[0][4]["initial_cash"], "2.0000");
        sqlx::query("DELETE FROM accounts WHERE id=$1")
            .bind(id)
            .execute(&db.owner)
            .await?;
        assert!(table_rows(&snapshot(&db).await?, "accounts").is_empty());
        Ok(())
    }
    .await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_catalog_grant_rls_migration_and_unknown_inventory() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        let before=snapshot(&db).await?;
        sqlx::raw_sql("REVOKE SELECT ON public.accounts FROM admin").execute(&db.owner).await?;
        assert_ne!(table_rows(&snapshot(&db).await?,"pg_class"),table_rows(&before,"pg_class"));
        sqlx::raw_sql("ALTER TABLE public.accounts NO FORCE ROW LEVEL SECURITY").execute(&db.owner).await?;
        assert!(snapshot(&db).await.is_err());
        sqlx::raw_sql("ALTER TABLE public.accounts FORCE ROW LEVEL SECURITY").execute(&db.owner).await?;
        // The current reader rejects altered migration bytes before emitting rows.
        // Restore each original so later inventory negatives exercise their own guard.
        for version in [1_i64,68,69] {
            let checksum: Vec<u8> = sqlx::query_scalar("SELECT checksum FROM public._sqlx_migrations WHERE version=$1")
                .bind(version).fetch_one(&db.owner).await?;
            sqlx::query("UPDATE public._sqlx_migrations SET checksum=decode(repeat('ab',48),'hex') WHERE version=$1")
                .bind(version).execute(&db.owner).await?;
            let error=snapshot(&db).await.expect_err("altered migration bytes must be rejected");
            assert_eq!(error.to_string(),"R5_PAPER_STAGE_FAILED:AUDIT_READ SQLSTATE=23514 GUARD=UNKNOWN");
            sqlx::query("UPDATE public._sqlx_migrations SET checksum=$1 WHERE version=$2")
                .bind(checksum).bind(version).execute(&db.owner).await?;
            assert_eq!(table_rows(&snapshot(&db).await?,"_sqlx_migrations"),table_rows(&before,"_sqlx_migrations"));
        }
        sqlx::raw_sql("CREATE TABLE public.unexpected_paper_audit_table(id integer)").execute(&db.owner).await?;
        assert!(snapshot(&db).await.is_err());
        sqlx::raw_sql("DROP TABLE public.unexpected_paper_audit_table").execute(&db.owner).await?;
        snapshot(&db).await?;
        sqlx::raw_sql("ALTER TABLE public.accounts ADD COLUMN unexpected integer").execute(&db.owner).await?;
        assert!(snapshot(&db).await.is_err());
        Ok(())
    }.await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_exact_receipts_and_legacy_null_history() -> Daily0060Result {
    for legacy in [false, true] {
        let db =
            Daily0060Db::new(require_r5_03_database_url()?, if legacy { 67 } else { 69 }).await?;
        let result: Daily0060Result = async {
            configure(&db).await?;
            if legacy {
                // Current reader rejects the old schema before the forward upgrade.
                assert!(snapshot(&db).await.is_err());
                MIGRATOR.run_to(68,&db.owner).await?;
                assert!(snapshot(&db).await.is_err());
                MIGRATOR.run_to(69,&db.owner).await?;
            }
            let rows=snapshot(&db).await?;
            let events=table_rows(&rows,"owner_beta_paper_events");
            let configured=events.iter().find(|r|r[4]["kind"]=="CONFIGURED").ok_or("configured event missing")?;
            let original:Value=sqlx::query_scalar("SELECT jsonb_build_object('request_canonical',request_canonical,'request_sha256',request_sha256,'result_canonical',result_canonical,'result_sha256',result_sha256) FROM owner_beta_paper_events WHERE kind='CONFIGURED'").fetch_one(&db.owner).await?;
            for name in ["request_canonical","request_sha256","result_canonical","result_sha256"] {
                assert_eq!(configured[4][name],original[name]);
                assert_eq!(configured[4][name].is_null(),legacy);
            }
            Ok(())
        }.await;
        db.close().await?;
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_relationship_orphan_and_redacted_delivery() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        let notification:Uuid=sqlx::query_scalar("INSERT INTO notifications(owner_user_id,kind,title,body) VALUES($1,'job','fixture','fixture') RETURNING id").bind(db.owner_id).fetch_one(&db.owner).await?;
        sqlx::query("INSERT INTO notification_deliveries(notification_id,owner_user_id,channel,status,error_detail) VALUES($1,$2,'web','FAILED','DO_NOT_DISCLOSE_AUDIT_SENTINEL')").bind(notification).bind(db.owner_id).execute(&db.owner).await?;
        let before=snapshot(&db).await?;
        let deliveries=table_rows(&before,"notification_deliveries");
        assert_eq!(deliveries.len(),1);
        let expected:String=sqlx::query_scalar("SELECT 'sha256:'||encode(sha256(convert_to('DO_NOT_DISCLOSE_AUDIT_SENTINEL','UTF8')),'hex')").fetch_one(&db.owner).await?;
        assert_eq!(deliveries[0][4]["error_detail"],expected);
        assert!(!serde_json::to_string(&before)?.contains("DO_NOT_DISCLOSE_AUDIT_SENTINEL"));
        // Remove FK enforcement only inside this disposable fixture. The real audit
        // must independently reject the orphan; no serving RLS bypass is enabled.
        sqlx::raw_sql("ALTER TABLE notification_deliveries DROP CONSTRAINT notification_deliveries_notification_id_fkey, DROP CONSTRAINT notification_deliveries_notification_owner_fk").execute(&db.owner).await?;
        sqlx::query("DELETE FROM notifications WHERE id=$1").bind(notification).execute(&db.owner).await?;
        assert!(snapshot(&db).await.is_err());
        Ok(())
    }.await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_cancellation_restores_actor_at_caller_savepoint() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let result: Daily0060Result = async {
        let mut blocker = db.owner.begin().await?;
        sqlx::raw_sql("LOCK TABLE accounts IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        let mut c = db.owner.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *c)
            .await?;
        let actor = db.owner_id.to_string();
        sqlx::raw_sql("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SAVEPOINT audit_call")
            .execute(&mut *c)
            .await?;
        let task = tokio::spawn(async move {
            let result = sqlx::raw_sql(AUDIT_SQL).fetch_all(&mut *c).await;
            let error = result.expect_err("audit should be canceled while blocked");
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("57014")
            );
            sqlx::raw_sql("ROLLBACK TO SAVEPOINT audit_call")
                .execute(&mut *c)
                .await?;
            let observed: String =
                sqlx::query_scalar("SELECT current_setting('app.actor_user_id')")
                    .fetch_one(&mut *c)
                    .await?;
            assert_eq!(observed, actor);
            sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
            Ok::<(), sqlx::Error>(())
        });
        let waited = wait_for_pg_blocker(&db, pid, blocker_pid).await;
        let canceled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
            .bind(pid)
            .fetch_one(&db.owner)
            .await?;
        blocker.rollback().await?;
        task.await??;
        waited?;
        assert!(canceled);
        Ok(())
    }
    .await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh isolated PG and process-only fixture clock"]
async fn audit_daily_source_settlement_valuation_and_missing_economics() -> Daily0060Result {
    let db = isolated_db().await?;
    MIGRATOR.run_to(69, &db.owner).await?;
    let clock = FixtureClock::new().await?;
    let initial = clock.after_cutoff().await?;
    let result: Daily0060Result = async {
        let f = r5_sql_stage("AUDIT_SETUP", enabled_paper(&db, true).await)?;
        let (target, _) = r5_sql_stage("AUDIT_TARGET", auto_target(&db, &f, true).await)?;
        let effective: String =
            sqlx::query_scalar("SELECT effective_date::text FROM pending_targets WHERE id=$1")
                .bind(target)
                .fetch_one(&db.owner)
                .await?;
        clock.set(initial + 86400).await?;
        let e = r5_sql_stage(
            "AUDIT_E_SOURCE",
            seed_witness(
                &db,
                f.config,
                &f.cohort,
                &effective,
                Some(&f.witness.admission),
                true,
            )
            .await,
        )?;
        let settlement = r5_sql_stage(
            "AUDIT_SETTLE",
            settlement_packet(&db, &f, target, &e.admission, true).await,
        )?;
        assert_eq!(
            r5_sql_stage("AUDIT_SETTLE", settle(&db, &f, target, &settlement).await)?["outcome"],
            "OK"
        );
        let valuation = r5_sql_stage(
            "AUDIT_VALUE",
            valuation_packet(
                &db,
                &f,
                target,
                &e.admission,
                "10100.0000",
                "9999900.0000",
                "101000.0000",
            )
            .await,
        )?;
        assert_eq!(
            r5_sql_stage("AUDIT_VALUE", valuate(&db, &f, target, &valuation).await)?["outcome"],
            "OK"
        );
        r5_sql_stage("AUDIT_UPGRADE", MIGRATOR.run_to(68, &db.owner).await)?;
        let rows = snapshot(&db).await?;
        let sessions = table_rows(&rows, "owner_beta_paper_sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0][4]["receipt_json"]["settlement"], settlement);
        assert_eq!(sessions[0][4]["receipt_json"]["valuation"], valuation);
        let targets = table_rows(&rows, "pending_targets");
        let lineages = table_rows(&rows, "owner_beta_paper_lineage");
        assert_eq!(targets.len(), 1);
        assert_eq!(lineages.len(), 1);
        assert_eq!(targets[0][4]["beta_lineage_id"], lineages[0][4]["id"]);
        assert_eq!(targets[0][4]["source_kind"], "OWNER_BETA_DAILY_AUTO");
        assert_eq!(
            lineages[0][4]["recommendation_run_id"],
            f.witness.run.to_string()
        );
        for key in [
            "recommendation_run_id",
            "dataset_version",
            "dataset_version_id",
            "dataset_manifest_sha256",
        ] {
            assert!(
                targets[0][4][key].is_null(),
                "beta target retains generic source exclusion"
            );
        }
        let equity = table_rows(&rows, "daily_equity");
        assert_eq!(equity.len(), 1);
        assert_eq!(equity[0][4]["equity"], "9999900.0000");
        assert_eq!(equity[0][4]["cash"], "9898900.0000");
        assert_eq!(equity[0][4]["positions_value"], "101000.0000");
        for table in [
            "owner_beta_daily_admissions",
            "owner_beta_daily_reservations",
            "owner_beta_daily_publication_claims",
            "owner_beta_paper_source_slots",
            "owner_beta_paper_source_admissions",
            "owner_beta_paper_source_runs",
            "orders",
            "fills",
            "cash_ledger",
            "paper_settlement_outbox",
        ] {
            assert!(
                !table_rows(&rows, table).is_empty(),
                "fixture table {table}"
            );
        }
        // Migration-owner tampering is visible to the actual audit without
        // disabling any FK/trigger. Restore the exact target projection after
        // the negative so the separate missing-economic-row check still runs.
        let target_weights = targets[0][4]["targets_json"].clone();
        assert!(
            target_weights
                .as_array()
                .is_some_and(|weights| !weights.is_empty())
        );
        sqlx::query("UPDATE pending_targets SET targets_json='[]'::jsonb WHERE id=$1")
            .bind(target)
            .execute(&db.owner)
            .await?;
        let relationship = snapshot(&db)
            .await
            .expect_err("target allocation must match its original beta lineage");
        assert_eq!(
            relationship.to_string(),
            "R5_PAPER_STAGE_FAILED:AUDIT_READ SQLSTATE=23514 GUARD=paper_audit_session"
        );
        sqlx::query("UPDATE pending_targets SET targets_json=$2 WHERE id=$1")
            .bind(target)
            .bind(target_weights)
            .execute(&db.owner)
            .await?;
        denied(
            sqlx::query("UPDATE pending_targets SET computed_on=computed_on-1 WHERE id=$1")
                .bind(target)
                .execute(&db.owner)
                .await,
            "23503",
        );
        assert!(
            table_rows(&snapshot(&db).await?, "pending_targets") == targets,
            "rejected relationship mutation and restored allocation preserve original target"
        );
        sqlx::query("DELETE FROM daily_equity WHERE account_id=$1")
            .bind(f.account)
            .execute(&db.owner)
            .await?;
        let missing = snapshot(&db)
            .await
            .expect_err("valued session cannot hide missing economic row");
        assert_eq!(
            missing.to_string(),
            "R5_PAPER_STAGE_FAILED:AUDIT_READ SQLSTATE=23514 GUARD=paper_audit_valuation_rows"
        );
        Ok(())
    }
    .await;
    clock.set(0).await?;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_role_flags_and_membership_catalog_changes() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 69).await?;
    let admin = admin_pool(&db.url)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let role = format!("paper_audit_probe_{}", Uuid::new_v4().simple());
    // A throwaway no-login role, with no object grants. Never elevate the audit
    // principal or use the admin connection to serve an audit.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role} NOLOGIN NOINHERIT"
    )))
    .execute(&admin)
    .await?;
    let result: Daily0060Result = async {
        let before = snapshot(&db).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER ROLE {role} INHERIT CONNECTION LIMIT 3; GRANT {role} TO migration_owner"
        )))
        .execute(&admin)
        .await?;
        let after = snapshot(&db).await?;
        if table_rows(&before, "pg_roles") == table_rows(&after, "pg_roles")
            || table_rows(&before, "pg_auth_members") == table_rows(&after, "pg_auth_members")
        {
            return Err("audit omitted role flags or role membership".into());
        }
        Ok(())
    }
    .await;
    let cleanup = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE {role} FROM migration_owner; DROP ROLE {role}"
    )))
    .execute(&admin)
    .await;
    admin.close().await;
    db.close().await?;
    cleanup?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_historical_reader_private_catalog_and_pristine_down() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let result: Daily0060Result = async {
        let public_reader: String = sqlx::query_scalar(
            "SELECT pg_get_functiondef('public.read_owner_beta_paper_session_evidence(date,jsonb)'::regprocedure)",
        ).fetch_one(&db.owner).await?;
        let catalog: (String, bool, Vec<String>) = sqlx::query_as(
            "SELECT pg_get_userbyid(proowner)::text,prosecdef,proconfig FROM pg_proc WHERE oid='public.paper_0068_read_recovery_session(jsonb,date,jsonb)'::regprocedure",
        ).fetch_one(&db.owner).await?;
        assert_eq!(catalog.0, "migration_owner");
        assert!(catalog.1);
        for setting in ["search_path=pg_catalog, pg_temp", "lock_timeout=5s", "statement_timeout=15s"] {
            assert!(catalog.2.iter().any(|actual| actual == setting), "{setting}");
        }
        let grants: Vec<String> = sqlx::query_scalar(
            "SELECT CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee)::text END FROM pg_proc p CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE p.oid='public.paper_0068_read_recovery_session(jsonb,date,jsonb)'::regprocedure AND a.privilege_type='EXECUTE' ORDER BY 1",
        ).fetch_all(&db.owner).await?;
        assert_eq!(grants, vec!["migration_owner".to_owned()]);
        let worker = worker_without_actor(&db, 1).await?;
        denied(sqlx::query_scalar::<_, Value>(
            "SELECT public.paper_0068_read_recovery_session(NULL,NULL,NULL)",
        ).fetch_one(&worker).await, "42501");
        worker.close().await;
        let mut tx = db.owner.begin().await?;
        sqlx::raw_sql(PAPER_0068_DOWN).execute(&mut *tx).await?;
        let removed: Option<String> = sqlx::query_scalar(
            "SELECT to_regprocedure('public.paper_0068_read_recovery_session(jsonb,date,jsonb)')::text",
        ).fetch_one(&mut *tx).await?;
        assert!(removed.is_none());
        let unchanged: String = sqlx::query_scalar(
            "SELECT pg_get_functiondef('public.read_owner_beta_paper_session_evidence(date,jsonb)'::regprocedure)",
        ).fetch_one(&mut *tx).await?;
        assert_eq!(public_reader, unchanged, "0066 public authority ABI/source never changes");
        tx.rollback().await?;
        Ok(())
    }.await;
    db.close().await?;
    result
}

#[tokio::test]
#[ignore = "requires Root's fresh R5-03 disposable PostgreSQL manifest"]
async fn audit_worker_recovery_reader_private_role_and_pristine_down() -> Daily0060Result {
    let db = Daily0060Db::new(require_r5_03_database_url()?, 68).await?;
    let result:Daily0060Result=async {
        let signature="public.paper_0068_read_worker_recovery(uuid,uuid,bigint,text,text,date,text)";
        let (owner,definer,settings):(String,bool,Vec<String>)=sqlx::query_as(
            "SELECT pg_get_userbyid(proowner)::text,prosecdef,proconfig FROM pg_proc WHERE oid=$1::regprocedure",
        ).bind(signature).fetch_one(&db.owner).await?;
        assert_eq!(owner,"migration_owner");assert!(definer);
        for expected in ["search_path=pg_catalog, pg_temp","lock_timeout=5s","statement_timeout=15s"] {assert!(settings.iter().any(|s|s==expected));}
        for (role,expected) in [("worker",true),("app",false),("admin",false),("audit_writer",false),("research_writer",false),("daily_input_admitter",false)] {
            let allowed:bool=sqlx::query_scalar("SELECT has_function_privilege($1,$2,'EXECUTE')").bind(role).bind(signature).fetch_one(&db.owner).await?;
            assert_eq!(allowed,expected,"{role}");
        }
        let broad:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_proc p CROSS JOIN LATERAL aclexplode(p.proacl) a WHERE p.oid=$1::regprocedure AND a.grantee=0)")
            .bind(signature).fetch_one(&db.owner).await?;assert!(!broad);
        let native:bool=sqlx::query_scalar("SELECT has_function_privilege('worker','public.paper_0068_read_recovery_session(jsonb,date,jsonb)','EXECUTE')").fetch_one(&db.owner).await?;
        assert!(!native,"worker cannot use migration-owner native reader");
        let original:String=sqlx::query_scalar("SELECT pg_get_functiondef('public.read_owner_beta_paper_session_evidence(date,jsonb)'::regprocedure)").fetch_one(&db.owner).await?;
        let mut tx=db.owner.begin().await?;
        sqlx::raw_sql(PAPER_0068_DOWN).execute(&mut *tx).await?;
        let absent:Option<String>=sqlx::query_scalar("SELECT to_regprocedure($1)::text").bind(signature).fetch_one(&mut *tx).await?;assert!(absent.is_none());
        let restored:String=sqlx::query_scalar("SELECT pg_get_functiondef('public.read_owner_beta_paper_session_evidence(date,jsonb)'::regprocedure)").fetch_one(&mut *tx).await?;
        assert_eq!(original,restored);tx.rollback().await?;Ok(())
    }.await;
    db.close().await?;
    result
}
