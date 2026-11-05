use super::{Daily0060Db, Daily0060Result, MIGRATOR, admin_pool};
use serde_json::{Value, json};
use sqlx::Acquire;
use sqlx::postgres::PgPool;
use sqlx::types::Uuid;
use std::env;

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
        if !(0..=259200).contains(&seconds)
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
    sqlx::query("INSERT INTO owner_beta_paper_source_slots(slot_id,reservation_id,root_event_sha256,owner_user_id,policy_sha256,activation_epoch,session_date,cohort_sha256,cohort) VALUES($1,$2,$3,$4,$5,($6::jsonb->>'activation_epoch')::bigint,$7::text::date,$8,$6)").bind(&slot).bind(Uuid::new_v4()).bind(&root).bind(db.owner_id).bind(hash('b')).bind(cohort).bind(session).bind(hash('d')).execute(&mut *tx).await?;
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

async fn enabled_paper(db: &Daily0060Db, weight: bool) -> Daily0060Result<SqlPaper> {
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
        "SELECT clock_timestamp()::text,(clock_timestamp()+interval '120 seconds')::text",
    )
    .fetch_one(&db.owner)
    .await?;
    let evidence = json!({"current_cohort":cohort,"process_binding":binding,"process_binding_sha256":digest,"evaluated_at":now,"expires_at":expires,"expected_epoch":0,"new_epoch":1,"session_evidence":witnesses.iter().map(|w|json!({"session_date":w.session,"run_id":w.run.to_string(),"admission_sha256":w.admission,"source_event_sha256":w.source,"root_event_sha256":w.root})).collect::<Vec<_>>()});
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
    Ok(SqlPaper {
        account,
        config,
        cohort,
        digest,
        witness: witnesses.pop().ok_or("witness")?,
    })
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
                'owner_beta_paper_session_events'])
        ), functions AS (
            SELECT p.* FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
            WHERE n.nspname='public' AND (
                p.proname LIKE 'paper_0067_%' OR p.proname LIKE '%owner_beta_paper%'
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
            "SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_roles r ON r.oid=c.relowner WHERE n.nspname='public' AND c.relname=ANY(ARRAY['owner_beta_paper_control','owner_beta_paper_events','owner_beta_paper_lineage','owner_beta_paper_sessions','owner_beta_paper_session_events']) AND r.rolname='migration_owner' AND c.relrowsecurity AND c.relforcerowsecurity",
        )
        .fetch_one(&db.owner)
        .await?;
        if table_count != 5 {
            return Err("all five beta ledger tables must be migration-owned and FORCE RLS".into());
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

        let mut tx = db.owner.begin().await?;
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

fn denied<T>(result: Result<T, sqlx::Error>, code: &str) {
    let error = result.err().expect("operation must be denied");
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some(code),
        "{error}"
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
        super::daily_0060_history(&db).await?;
        let f = enabled_paper(&db, true).await?;
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

        let mut trial = db.worker.begin().await?;
        let trial_result: Value = sqlx::query_scalar(
            "SELECT apply_owner_beta_paper_preview_auto_v1($1,1,$2)",
        )
        .bind(t2_preview)
        .bind(&f.digest)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(trial_result["outcome"], "OK", "unrevoked READY retry reaches apply");
        let applied_preview: Value = sqlx::query_scalar(
            "SELECT to_jsonb(p) FROM paper_rebalance_previews p WHERE p.id=$1",
        )
        .bind(t2_preview)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(applied_preview["status"], "APPLIED");
        let trial_target = Uuid::parse_str(
            applied_preview["pending_target_id"]
                .as_str()
                .ok_or("trial T2 target")?,
        )?;
        assert_ne!(trial_target, t1);
        let trial_effective: String = sqlx::query_scalar(
            "SELECT effective_date::text FROM pending_targets WHERE id=$1",
        )
        .bind(trial_target)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(trial_effective, e2_date, "trial retry creates the E2 target");
        let trial_rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM pending_targets WHERE id=$1),(SELECT count(*) FROM owner_beta_paper_sessions WHERE pending_target_id=$1)",
        )
        .bind(trial_target)
        .fetch_one(&mut *trial)
        .await?;
        assert_eq!(trial_rows, (1, 1), "trial target has its real session row");
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
        let bounded = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            sqlx::query_scalar::<_, Value>("SELECT apply_owner_beta_paper_preview_v1($1,$2,1,$3)")
                .bind(preview)
                .bind(packet["preview_token"].as_str().ok_or("manual token")?)
                .bind(&f.digest)
                .fetch_one(&db.app),
        )
        .await
        .expect("source-lock wait must terminate within the SQL lock bound");
        denied(bounded, "55P03");
        exclusive.rollback().await?;
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
        let mut tx=db.owner.begin().await?;
        sqlx::query("SELECT set_config('app.actor_user_id',$1,true)").bind(foreign.to_string()).execute(&mut *tx).await?;
        sqlx::query("UPDATE jobs SET status='CANCELED',error_code='canceled',finished_at=clock_timestamp() WHERE id=$1 AND owner_user_id=$2 AND job_type='owner_beta_paper_preview' AND status='QUEUED'")
            .bind(job).bind(db.owner_id).execute(&mut *tx).await?;
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
            sqlx::query("INSERT INTO jobs(id,owner_user_id,job_type,status,payload_json) VALUES($1,$2,'owner_beta_paper_preview','QUEUED',$3)")
                .bind(fake_job)
                .bind(owner)
                .bind(payload)
                .execute(&db.owner)
                .await?;
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
