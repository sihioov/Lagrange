//! Synthetic SQL fixtures only. They never create an approved filesystem capability.
//! The policy and evidence fixture shape are preserved from the accepted 0060
//! migration-contract fixture; only connection ownership is adapted here.

use crate::common::ScratchDb;
use chrono::NaiveDate;
use serde_json::Value;
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{error::Error, future::Future, sync::Arc, time::Duration};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

pub struct DailyFixture {
    pub db: ScratchDb,
    pub owner_id: Uuid,
    pub worker: PgPool,
    pub admitter: PgPool,
    pub app: PgPool,
}

pub async fn run<F, Fut>(case: &'static str, body: F)
where
    F: FnOnce(Arc<DailyFixture>) -> Fut + Send + 'static,
    Fut: Future<Output = TestResult> + Send + 'static,
{
    let Some(db) = ScratchDb::create().await else {
        eprintln!("SKIP: daily runtime DB case {case}");
        return;
    };
    let owner_id = Uuid::new_v4();
    let setup = async {
        sqlx::query("INSERT INTO users(id,issuer,subject,email) VALUES($1,'https://daily.invalid','daily-runtime','daily-runtime@example.invalid')")
            .bind(owner_id).execute(&db.pool).await?;
        sqlx::query("INSERT INTO roles(id) VALUES('owner') ON CONFLICT DO NOTHING").execute(&db.pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id) VALUES($1,'owner')").bind(owner_id).execute(&db.pool).await?;
        sqlx::query("INSERT INTO strategies(id,display_name) VALUES('daily_fixture','Daily fixture')").execute(&db.pool).await?;
        for instrument in daily_0060_policy()["instruments"].as_array().ok_or("instruments")? {
            sqlx::query("INSERT INTO instruments(id,symbol,venue,currency) VALUES($1,split_part($1,'.',1),'KRX','KRW') ON CONFLICT DO NOTHING")
                .bind(instrument.as_str().ok_or("instrument")?).execute(&db.pool).await?;
        }
        sqlx::query("INSERT INTO data_entitlements(contract_document_sha256,contract_reference,status,covered_datasets,covered_uses,effective_from,effective_until,managed_by) VALUES($1,'daily-fixture','ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\"]','2016-08-29','2099-12-31',$2)")
            .bind("a".repeat(64)).bind(owner_id).execute(&db.pool).await?;
        let worker = role_pool(&db,"worker",None).await?;
        let admitter = role_pool(&db,"daily_input_admitter",None).await?;
        let app = role_pool(&db,"app",Some(owner_id)).await?;
        Ok::<_, Box<dyn Error + Send + Sync>>((worker,admitter,app))
    }.await;
    let (worker, admitter, app) = match setup {
        Ok(pools) => pools,
        Err(_) => {
            drop_checked(db).await;
            panic!("daily runtime fixture setup failed");
        }
    };
    let fixture = Arc::new(DailyFixture {
        db,
        owner_id,
        worker,
        admitter,
        app,
    });
    let task_fixture = fixture.clone();
    let result = tokio::spawn(async move { body(task_fixture).await }).await;
    let fixture =
        Arc::try_unwrap(fixture).unwrap_or_else(|_| panic!("daily fixture still borrowed"));
    fixture.worker.close().await;
    fixture.admitter.close().await;
    fixture.app.close().await;
    drop_checked(fixture.db).await;
    if let Ok(Err(error)) = &result {
        if let Some(error) = error.downcast_ref::<sqlx::Error>() {
            if let Some(code) = error.as_database_error().and_then(|e| e.code())
                && code.len() == 5
                && code.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                eprintln!("DAILY_RUNTIME_CASE:{case}:SQLSTATE:{code}");
            }
        } else if let Some(error) =
            error.downcast_ref::<job_queue::owner_beta::runner::daily_runtime::DailyRuntimeError>()
        {
            eprintln!("DAILY_RUNTIME_CASE:{case}:RUNTIME:{error:?}");
        }
    }
    assert!(
        matches!(result, Ok(Ok(()))),
        "daily runtime DB case failed: {case}"
    );
}

async fn drop_checked(db: ScratchDb) {
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|_| panic!("scratch database identity unavailable before cleanup"));
    db.drop_db().await;
    let options: PgConnectOptions = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| panic!("scratch supervisor is not configured"))
        .parse()
        .unwrap_or_else(|_| panic!("scratch supervisor options are invalid"));
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.disable_statement_logging())
        .await
        .unwrap_or_else(|_| panic!("cleanup supervisor connection failed"));
    let absent = sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)",
    )
    .bind(name)
    .fetch_one(&admin)
    .await;
    admin.close().await;
    assert!(
        matches!(absent, Ok(true)),
        "scratch cleanup was not verified"
    );
}

async fn role_pool(db: &ScratchDb, role: &str, actor: Option<Uuid>) -> TestResult<PgPool> {
    let mut options: PgConnectOptions = db.role_url(role).parse()?;
    options = options.options([("statement_timeout", "15000"), ("lock_timeout", "7000")]);
    if let Some(actor) = actor {
        options = options.options([("app.actor_user_id", actor.to_string())]);
    }
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.disable_statement_logging())
        .await?;
    let ids: (String, String) = sqlx::query_as("SELECT current_user,session_user")
        .fetch_one(&pool)
        .await?;
    assert_eq!(ids.0, role);
    assert_eq!(ids.0, ids.1);
    Ok(pool)
}

impl DailyFixture {
    pub async fn control(&self, enabled: bool) -> TestResult {
        sqlx::query("INSERT INTO owner_beta_daily_control(owner_user_id,policy_sha256,policy_json,entitlement_reference,entitlement_sha256,first_auto_session,max_jobs_per_owner,lease_seconds,backoff_seconds) VALUES($1,$2,$3,'daily-fixture',$4,'2026-08-29',4,60,1)")
            .bind(self.owner_id).bind(daily_0060_hash_value('b')).bind(daily_0060_policy()).bind(daily_0060_hash_value('a')).execute(&self.db.pool).await?;
        if enabled {
            sqlx::query("UPDATE owner_beta_daily_control SET enabled=true")
                .execute(&self.db.pool)
                .await?;
        }
        Ok(())
    }
    pub async fn ready(&self) -> TestResult<Uuid> {
        self.control(true).await?;
        let config:Uuid=sqlx::query_scalar("INSERT INTO user_strategy_configs(owner_user_id,strategy_id,strategy_version,config_json,daily_enabled) VALUES($1,'daily_fixture','1.0.0','{}',true) RETURNING id")
            .bind(self.owner_id).fetch_one(&self.app).await?;
        let receipt: Uuid = sqlx::query_scalar(
            "SELECT receipt_id FROM public.take_owner_beta_daily_admission_poll_slot()",
        )
        .fetch_one(&self.admitter)
        .await?;
        let end:NaiveDate=sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date-CASE WHEN (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::time<time '16:30' THEN 1 ELSE 0 END").fetch_one(&self.db.pool).await?;
        let (input, file, days) = daily_0060_bundle(self, &end.to_string()).await?;
        let key:String=sqlx::query_scalar("SELECT daily_0060_digest('owner-beta-daily-calendar-source-v1'||chr(10)||owner_user_id::text||chr(10)||policy_sha256||chr(10)||'daily-raw-reconstruction-v1'||chr(10)||daily_0060_file_bytes($1)) FROM owner_beta_daily_control")
            .bind(&file).fetch_one(&self.db.pool).await?;
        let projected: String = sqlx::query_scalar(
            "SELECT disposition FROM public.project_owner_beta_daily_calendar($1,$2,$3,$4)",
        )
        .bind(receipt)
        .bind(key)
        .bind(file)
        .bind(days)
        .fetch_one(&self.admitter)
        .await?;
        assert_eq!(projected, "PROJECTED");
        let admitted: String = sqlx::query_scalar(
            "SELECT disposition FROM public.admit_owner_beta_daily_input($1,$2)",
        )
        .bind(receipt)
        .bind(input)
        .fetch_one(&self.admitter)
        .await?;
        assert_eq!(admitted, "ADMITTED");
        Ok(config)
    }
    pub async fn next_schedule(&self) -> TestResult {
        // Synthetic time fixture, release role only. Serving roles cannot do this.
        sqlx::query("UPDATE public.owner_beta_daily_poll_state SET next_poll_at=clock_timestamp()-interval '1 second' WHERE stage='SCHEDULE'")
            .execute(&self.db.pool).await?;
        Ok(())
    }
    pub async fn job_payloads(&self) -> TestResult<Vec<Value>> {
        Ok(sqlx::query_scalar("SELECT payload_json FROM public.jobs WHERE job_type='owner_beta_price_recommendation' ORDER BY id").fetch_all(&self.worker).await?)
    }
}

fn daily_0060_hash_value(digit: char) -> String {
    format!("sha256:{}", digit.to_string().repeat(64))
}

fn daily_0060_policy() -> serde_json::Value {
    use serde_json::json;
    json!({
        "schema":"owner-beta-daily-etf11-policy-v1","capability":"PRICE_RETURN_ONLY",
        "audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,
        "base":{"kind":"historical_price_only_v3","pins":{
            "candidate_content_sha256":daily_0060_hash_value('1'),"artifact_manifest_sha256":daily_0060_hash_value('2'),
            "stage5_manifest_sha256":daily_0060_hash_value('3'),"action_manifest_sha256":daily_0060_hash_value('4'),"approval_registry_sha256":daily_0060_hash_value('5')}},
        "instruments":["069500.KRX","102110.KRX","114260.KRX","132030.KRX","133690.KRX","143850.KRX","148070.KRX","153130.KRX","192090.KRX","195930.KRX","229200.KRX"],
        "price_semantics":"KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC","cash_action_policy":"CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1",
        "normalizer_version":"daily-raw-reconstruction-v1","materializer_version":"daily-price-only-v1",
        "source_kinds":[{"provider":"kis","market":"kr","kind":"bars"},{"provider":"kis","market":"kr","kind":"calendar"},
            {"provider":"kis","market":"kr","kind":"corporate_actions"},{"provider":"kis","market":"kr","kind":"reference"},{"provider":"kis-calendar","market":"kr","kind":"calendar"}],
        "limits":{"max_policy_bytes":262144,"max_manifest_bytes":16777216,"max_admission_bytes":1048576,"max_normalization_bytes":16777216,
            "max_bars_bytes":134217728,"max_sessions":20000,"max_civil_dates_per_addition":370,"max_action_chains_per_session":32768,
            "max_file_evidence_per_session":262144,"max_source_file_bytes":67108864,"max_normalized_document_bytes":67108864,
            "max_copy_depth":16,"max_pages_per_action_chain":10,"max_cash_rows_per_chain":100000,"max_json_depth":32}
    })
}

fn daily_0060_file_fixture(
    kind: &str,
    date: &str,
    name: &str,
    endpoint: &str,
    tr: &str,
    mut query: Vec<(&str, String)>,
) -> serde_json::Value {
    use serde_json::json;
    query.sort_by(|a, b| a.0.cmp(b.0));
    json!({"provider":"kis","market":"kr","batch_id":format!("00000000-0000-4000-8000-{}0060",date.replace('-',"")),"batch_date":date,
        "batch_json_sha256":daily_0060_hash_value('1'),"manifest_line_sha256":daily_0060_hash_value('2'),"file_name":name,"kind":kind,
        "wire_sha256":daily_0060_hash_value('3'),"size_bytes":1,"request":{"endpoint":format!("/uapi/domestic-stock/v1/{endpoint}"),
            "query":query,"headers":[["appkey","[REDACTED]"],["appsecret","[REDACTED]"],["authorization","[REDACTED]"],["tr_cont",""],["tr_id",tr]],"mode":"credentialed"},
        "response_continuation":null,"retrieved_at":"2026-08-29T00:00:00.000000000Z","retrieval_precision":"batch",
        "original_retrieved_at":"2026-08-29T00:00:00.000000000Z","original_retrieval_precision":"batch","copied_from":[]})
}

fn daily_0060_file_key_fixture(f: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({"provider":f["provider"],"market":f["market"],"batch_id":f["batch_id"],"file_name":f["file_name"],"wire_sha256":f["wire_sha256"]})
}

async fn daily_0060_bundle(
    db: &DailyFixture,
    end: &str,
) -> TestResult<(serde_json::Value, serde_json::Value, serde_json::Value)> {
    use serde_json::json;
    let dates: Vec<String>=sqlx::query_scalar("SELECT (date '2026-08-29'+n)::text FROM generate_series(0,$1::text::date-date '2026-08-29') n ORDER BY n")
        .bind(end).fetch_all(&db.db.pool).await?;
    if dates.is_empty() || dates.len() > 370 {
        return Err("fixture date out of bounded extension".into());
    }
    let base: Vec<String>=sqlx::query_scalar("SELECT d::text FROM (SELECT date '2016-08-29'+n d FROM generate_series(0,2450) n UNION ALL SELECT date '2026-08-28') x ORDER BY d")
        .fetch_all(&db.db.pool).await?;
    let calendar = daily_0060_file_fixture(
        "calendar",
        "2026-08-29",
        "calendar.json",
        "quotations/chk-holiday",
        "CTCA0903R",
        vec![
            ("BASS_DT", "20260829".into()),
            ("CTX_AREA_FK", String::new()),
            ("CTX_AREA_NK", String::new()),
        ],
    );
    let mut days = Vec::new();
    let mut bindings = Vec::new();
    let mut documents = Vec::new();
    for (ordinal, date) in dates.iter().enumerate() {
        let session_type = if date == end { "TRADING" } else { "CLOSED" };
        days.push(json!({"session_date":date,"session_type":session_type,"evidence_kind":if ordinal==0 {"REQUESTED_TARGET"} else {"DERIVED_SURROUNDING"},"source_row_ordinal":ordinal,"source_row_sha256":daily_0060_hash_value('4')}));
        bindings.push(json!({"civil_date":date,"session_type":session_type,"exchange":"KRX","timezone":"Asia/Seoul","source_file":calendar,"normalized_document_sha256":daily_0060_hash_value('5'),"legacy_projection":null}));
        documents.push(json!({"kind":"calendar","session_date":date,"normalizer_version":"daily-raw-reconstruction-v1","sha256":daily_0060_hash_value('5'),"size_bytes":1,"row_count":1,"source_files":[daily_0060_file_key_fixture(&calendar)],"source_evidence_sha256":daily_0060_hash_value('6')}));
    }
    let policy = daily_0060_policy();
    let mut bars = Vec::new();
    let mut refs = Vec::new();
    for instrument in policy["instruments"]
        .as_array()
        .ok_or("fixture instruments")?
    {
        let symbol = instrument
            .as_str()
            .ok_or("fixture symbol")?
            .split('.')
            .next()
            .ok_or("fixture symbol")?;
        bars.push(daily_0060_file_fixture(
            "bars",
            end,
            &format!("bars-{symbol}.json"),
            "quotations/inquire-daily-itemchartprice",
            "FHKST03010100",
            vec![
                ("FID_COND_MRKT_DIV_CODE", "J".into()),
                ("FID_INPUT_ISCD", symbol.into()),
                ("FID_INPUT_DATE_1", end.replace('-', "")),
                ("FID_INPUT_DATE_2", end.replace('-', "")),
                ("FID_PERIOD_DIV_CODE", "D".into()),
                ("FID_ORG_ADJ_PRC", "1".into()),
            ],
        ));
        refs.push(daily_0060_file_fixture(
            "reference",
            end,
            &format!("reference-{symbol}.json"),
            "quotations/inquire-price",
            "FHKST01010100",
            vec![
                ("FID_COND_MRKT_DIV_CODE", "J".into()),
                ("FID_INPUT_ISCD", symbol.into()),
            ],
        ));
    }
    let mut actions = Vec::new();
    let mut pages = Vec::new();
    for (class, path, tr, extras) in [
        (
            "paidin-subscription",
            "paidin-capin",
            "HHKDB669100C0",
            vec![("GB1", "1".into())],
        ),
        (
            "paidin-record",
            "paidin-capin",
            "HHKDB669100C0",
            vec![("GB1", "2".into())],
        ),
        ("bonus-issue", "bonus-issue", "HHKDB669101C0", vec![]),
        (
            "dividend",
            "dividend",
            "HHKDB669102C0",
            vec![("GB1", "0".into()), ("HIGH_GB", String::new())],
        ),
        ("merger-split", "merger-split", "HHKDB669104C0", vec![]),
        (
            "reverse-split",
            "rev-split",
            "HHKDB669105C0",
            vec![("MARKET_GB", "0".into())],
        ),
        ("capital-decrease", "cap-dcrs", "HHKDB669106C0", vec![]),
    ] {
        let mut query = vec![
            ("CTS", String::new()),
            ("F_DT", "20260829".into()),
            ("T_DT", end.replace('-', "")),
            ("SHT_CD", String::new()),
        ];
        query.extend(extras);
        let page = daily_0060_file_fixture(
            "corporate_actions",
            "2026-08-29",
            &format!("{class}-{end}.json"),
            &format!("ksdinfo/{path}"),
            tr,
            query,
        );
        actions.push(json!({"logical_class":class,"scope":"whole_market","symbol":null,"range_start":"2026-08-29","range_end":end,"pages":[page],"cash_dividends":[],"cash_dividend_rows_sha256":"sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"}));
        pages.push(page);
    }
    for (kind, files, rows) in [
        ("bars", &bars, 11),
        ("reference", &refs, 11),
        ("corporate_actions", &pages, 0),
    ] {
        documents.push(json!({"kind":kind,"session_date":end,"normalizer_version":"daily-raw-reconstruction-v1","sha256":daily_0060_hash_value('7'),"size_bytes":1,"row_count":rows,
            "source_files":files.iter().map(daily_0060_file_key_fixture).collect::<Vec<_>>(),"source_evidence_sha256":daily_0060_hash_value('8')}));
    }
    let mut sessions = base;
    sessions.push(end.to_owned());
    let manifest = json!({"schema":"owner-beta-daily-etf11-input-v1","capability":"PRICE_RETURN_ONLY","audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,
        "base":policy["base"],"parent_admission_sha256":null,"range_start":"2016-08-29","range_end":end,"instruments":policy["instruments"],"sessions":sessions,
        "daily_sources":[{"session_date":end,"previous_frontier":"2026-08-28","instruments":policy["instruments"],"bars":bars,"reference":refs,"calendar":bindings,"actions":actions,"normalized_documents":documents}],
        "bars":{"file":"bars.ndjson","sha256":daily_0060_hash_value('9'),"size_bytes":1,"row_count":26983},
        "normalization":{"file":"normalization.json","sha256":daily_0060_hash_value('9'),"size_bytes":1,"row_count":3+dates.len()},
        "policy_sha256":daily_0060_hash_value('b'),"materializer_version":"daily-price-only-v1","code_commit":"0".repeat(40),"content_sha256":daily_0060_hash_value('d')});
    let admission = json!({"schema":"owner-beta-daily-etf11-admission-v1","content_sha256":daily_0060_hash_value('d'),"manifest_sha256":daily_0060_hash_value('e'),"policy_sha256":daily_0060_hash_value('b'),
        "parent_admission_sha256":null,"base":policy["base"],"verifier_code_commit":"0".repeat(40),"normalizer_version":"daily-raw-reconstruction-v1","evidence_sha256":daily_0060_hash_value('f'),
        "counts":{"instrument_count":11,"session_count":2453,"bar_count":26983,"added_session_count":1,"source_file_count":30,"calendar_row_count":dates.len(),"action_chain_count":7,"action_page_count":7,"cash_dividend_row_count":0,"normalized_document_count":3+dates.len()}});
    Ok((
        json!({"admission_sha256":daily_0060_hash_value('0'),"source_frontier_sha256":daily_0060_hash_value('6'),"admission":admission,"manifest":manifest}),
        calendar,
        json!(days),
    ))
}
