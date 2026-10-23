//! Synthetic persistence fixtures only. No fixture creates approved Raw/file capability.
//! Production serving roles and trigger gates remain enabled.
use crate::common::{Harness, UserCtx, actor_pool};
use axum::http::StatusCode;
use chrono::NaiveDate;
use job_queue::owner_beta::runner::daily_runtime::DailyProcessMode;
use job_queue::owner_beta::{OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE, OwnerBetaStrategySnapshot};
use job_queue::{JobQueue, QueueConfig, SubmitJob};
use serde_json::{Value, json};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{error::Error, future::Future, sync::Arc, time::Duration};
use uuid::Uuid;

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;
pub const DAILY_PATH: &str = "/api/v1/recommendations/owner-beta/price-only/daily-state";
pub const RUNS_PATH: &str = "/api/v1/recommendations/owner-beta/price-only/runs";

pub fn assert_contract(name: &str, value: &Value) {
    let spec: Value =
        serde_json::from_str(include_str!("../../../../apps/api-server/openapi.json"))
            .expect("versioned API contract");
    let schema =
        json!({"$ref":format!("#/components/schemas/{name}"),"components":spec["components"]});
    let validator = jsonschema::validator_for(&schema).expect("response schema resolves");
    assert!(validator.is_valid(value), "HTTP response violates {name}");
}

pub async fn run<F, Fut>(case: &'static str, mode: DailyProcessMode, body: F)
where
    F: FnOnce(Arc<Harness>) -> Fut + Send + 'static,
    Fut: Future<Output = TestResult> + Send + 'static,
{
    let Some(mut h) = Harness::new().await else {
        eprintln!("SKIP: API daily DB case {case}");
        return;
    };
    h.restart_api_with_daily(mode).await;
    let name = h.db_name.clone();
    let h = Arc::new(h);
    let task = h.clone();
    let result = tokio::spawn(async move { body(task).await }).await;
    let h = Arc::try_unwrap(h).unwrap_or_else(|_| panic!("API daily harness still borrowed"));
    h.teardown().await;
    let options: PgConnectOptions = std::env::var("DATABASE_URL")
        .expect("scratch supervisor configured")
        .parse()
        .expect("scratch options valid");
    let supervisor = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.disable_statement_logging())
        .await
        .expect("cleanup connection");
    let absent = sqlx::query_scalar::<_, bool>(
        "SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)",
    )
    .bind(name)
    .fetch_one(&supervisor)
    .await;
    supervisor.close().await;
    assert!(
        matches!(absent, Ok(true)),
        "API daily scratch cleanup was not verified"
    );
    if let Ok(Err(error)) = &result
        && let Some(error) = error.downcast_ref::<sqlx::Error>()
        && let Some(code) = error.as_database_error().and_then(|v| v.code())
        && code.len() == 5
        && code.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        eprintln!("API_DAILY_CASE:{case}:SQLSTATE:{code}");
    }
    assert!(
        matches!(result, Ok(Ok(()))),
        "API daily case failed: {case}"
    );
}
pub async fn create(
    h: &Harness,
    user: &UserCtx,
    enabled: Option<bool>,
) -> TestResult<(Uuid, Value)> {
    let mut body = json!({"strategy_version":"1.0.0","config":{"lookback":200},"is_active":true});
    if let Some(enabled) = enabled {
        body["owner_beta_daily_enabled"] = json!(enabled);
    }
    let response = h
        .post(
            "/api/v1/strategies/buy_and_hold/configs",
            Some(user),
            true,
            body,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let value = Harness::body_json(response).await;
    assert_contract("StrategyConfig", &value);
    Ok((value["id"].as_str().ok_or("config id")?.parse()?, value))
}
pub async fn patch(
    h: &Harness,
    user: &UserCtx,
    id: Uuid,
    body: Value,
    key: &str,
) -> axum::response::Response {
    h.send(
        "PATCH",
        &format!("/api/v1/strategy-configs/{id}"),
        Some(user),
        true,
        None,
        Some(key),
        Some(body),
    )
    .await
}
pub async fn state(h: &Harness, id: Uuid) -> Value {
    let response = h
        .get(
            &format!("{DAILY_PATH}?strategy_config_id={id}"),
            Some(&h.owner),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = Harness::body_json(response).await;
    assert_contract("OwnerBetaDailyState", &value);
    value
}
pub async fn control(h: &Harness, enabled: bool) -> TestResult {
    sqlx::query("INSERT INTO data_entitlements(contract_document_sha256,contract_reference,status,covered_datasets,covered_uses,effective_from,effective_until,managed_by) VALUES($1,'daily-fixture','ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\"]','2016-08-29','2099-12-31',$2)")
  .bind("a".repeat(64)).bind(h.owner.user_id).execute(&h.owner_pool).await?;
    for instrument in daily_0060_policy()["instruments"]
        .as_array()
        .ok_or("instruments")?
    {
        sqlx::query("INSERT INTO instruments(id,symbol,venue,currency) VALUES($1,split_part($1,'.',1),'KRX','KRW') ON CONFLICT DO NOTHING")
   .bind(instrument.as_str().ok_or("instrument")?).execute(&h.owner_pool).await?;
    }
    sqlx::query("INSERT INTO owner_beta_daily_control(owner_user_id,policy_sha256,policy_json,entitlement_reference,entitlement_sha256,first_auto_session,max_jobs_per_owner,lease_seconds,backoff_seconds) VALUES($1,$2,$3,'daily-fixture',$4,'2026-08-29',4,60,1)")
  .bind(h.owner.user_id).bind(daily_0060_hash_value('b')).bind(daily_0060_policy()).bind(daily_0060_hash_value('a')).execute(&h.owner_pool).await?;
    if enabled {
        sqlx::query("UPDATE owner_beta_daily_control SET enabled=true")
            .execute(&h.owner_pool)
            .await?;
    }
    Ok(())
}
pub async fn ready_input(h: &Harness) -> TestResult<NaiveDate> {
    control(h, true).await?;
    let end:NaiveDate=sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date-CASE WHEN (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::time<time '16:30' THEN 1 ELSE 0 END").fetch_one(&h.owner_pool).await?;
    let (input, file, days) = daily_0060_bundle(&h.owner_pool, &end.to_string()).await?;
    let admitter = h.daily_input_admitter_pool().await;
    let result=async {
  let receipt:Uuid=sqlx::query_scalar("SELECT receipt_id FROM public.take_owner_beta_daily_admission_poll_slot()").fetch_one(&admitter).await?;
  let key:String=sqlx::query_scalar("SELECT daily_0060_digest('owner-beta-daily-calendar-source-v1'||chr(10)||owner_user_id::text||chr(10)||policy_sha256||chr(10)||'daily-raw-reconstruction-v1'||chr(10)||daily_0060_file_bytes($1)) FROM owner_beta_daily_control")
   .bind(&file).fetch_one(&h.owner_pool).await?;
  let projected:String=sqlx::query_scalar("SELECT disposition FROM public.project_owner_beta_daily_calendar($1,$2,$3,$4)")
   .bind(receipt).bind(key).bind(file).bind(days).fetch_one(&admitter).await?;
  assert_eq!(projected,"PROJECTED");
  let admitted:String=sqlx::query_scalar("SELECT disposition FROM public.admit_owner_beta_daily_input($1,$2)")
   .bind(receipt).bind(input).fetch_one(&admitter).await?;
  assert_eq!(admitted,"ADMITTED");
  Ok::<_,Box<dyn Error+Send+Sync>>(end)
 }.await;
    admitter.close().await;
    result
}
pub async fn reserve(h: &Harness) -> TestResult<(PgPool, JobQueue, Uuid)> {
    let worker = h.worker_pool().await;
    let outcome = job_queue::owner_beta::runner::daily_schedule::schedule_daily_once(
        &worker,
        DailyProcessMode::OwnerOnly,
    )
    .await?;
    assert!(matches!(
        outcome,
        job_queue::owner_beta::runner::daily_schedule::DailyScheduleOutcome::Completed(_)
    ));
    let run: Uuid = sqlx::query_scalar("SELECT run_id FROM owner_beta_daily_reservations")
        .fetch_one(&h.owner_pool)
        .await?;
    let queue = JobQueue::new(
        worker.clone(),
        None,
        QueueConfig {
            lease: Duration::from_secs(60),
            backoff_base: Duration::from_secs(1),
        },
    );
    Ok((worker, queue, run))
}
/// Exercise SQL publication and serving reads with synthetic values. This is
/// deliberately not the artifact/computation acceptance path.
pub async fn publish_fixture(h: &Harness, worker: &PgPool, queue: &JobQueue) -> TestResult<Uuid> {
    let claim = queue
        .claim_next_for(
            "api-daily-fixture",
            OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE,
        )
        .await?
        .ok_or("claim")?;
    let run: Uuid = claim.job.payload_json["run_id"]
        .as_str()
        .ok_or("run")?
        .parse()?;
    let mut tx = worker.begin().await?;
    queue.lock_claim_in(&mut tx, &claim).await?;
    let allowed: bool = sqlx::query_scalar("SELECT public.lock_owner_beta_daily_run_gate($1)")
        .bind(run)
        .fetch_one(&mut *tx)
        .await?;
    assert!(allowed);
    for (index, instrument) in market_data::KR_ETF_CORE_SYMBOLS.iter().enumerate() {
        let selected = index == 0;
        let reason = if selected {
            "BENCHMARK_HELD"
        } else {
            "NOT_SELECTED_BY_STRATEGY"
        };
        sqlx::query("INSERT INTO owner_beta_recommendation_items(recommendation_run_id,owner_user_id,instrument_id,rank,target_weight,reason_codes,factors_json,excluded,exclusion_reason) VALUES($1,$2,$3,$4,$5::numeric,$6,'{}',$7,$8)")
   .bind(run).bind(h.owner.user_id).bind(format!("{instrument}.KRX"))
   .bind(selected.then_some(1_i32)).bind(if selected {"1.000000"} else {"0.000000"})
   .bind(json!([reason])).bind(!selected).bind((!selected).then_some(reason))
   .execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE owner_beta_recommendation_runs SET status='SUCCEEDED',factor_snapshot_sha256=$2,target_snapshot_sha256=$3,cash_weight=0,error_code=NULL WHERE id=$1")
  .bind(run).bind(daily_0060_hash_value('7')).bind(daily_0060_hash_value('8')).execute(&mut *tx).await?;
    let settled = queue.settle_success_in(&mut tx, &claim).await?;
    assert!(
        matches!(settled,job_queue::SettleResult::Committed(job) if job.status==job_queue::JobStatus::Succeeded)
    );
    tx.commit().await?;
    Ok(run)
}
pub async fn historical(h: &Harness, config: Uuid) -> TestResult<Uuid> {
    let app = actor_pool(&h.app_url, &h.owner.user_id.to_string(), 2).await;
    let strategy =
        OwnerBetaStrategySnapshot::from_resolved_config(&job_queue::resolver::ResolvedConfig {
            strategy_id: "buy_and_hold".into(),
            strategy_version: "1.0.0".into(),
            config: json!({"lookback":200}),
        })?;
    let run = Uuid::new_v4();
    let pins = json!({"candidate_content_sha256":daily_0060_hash_value('1'),"artifact_manifest_sha256":daily_0060_hash_value('2'),"stage5_manifest_sha256":daily_0060_hash_value('3'),"action_manifest_sha256":daily_0060_hash_value('4'),"approval_registry_sha256":daily_0060_hash_value('5')});
    let payload = json!({"run_id":run,"strategy_config_id":config,"as_of":"2026-08-28","pins":pins,"strategy":strategy});
    let queue = JobQueue::new(app.clone(), None, QueueConfig::default());
    let job = queue
        .submit(SubmitJob {
            owner_user_id: h.owner.user_id,
            job_type: OWNER_BETA_PRICE_RECOMMENDATION_JOB_TYPE.into(),
            payload,
            priority: 0,
            idempotency_key: Some(format!("api-manual-{run}")),
            max_attempts: 2,
            available_at: None,
        })
        .await?;
    sqlx::query("INSERT INTO owner_beta_recommendation_runs(id,owner_user_id,strategy_config_id,strategy_id,strategy_version,strategy_config_json,strategy_config_sha256,job_id,as_of,candidate_content_sha256,artifact_manifest_sha256,stage5_manifest_sha256,action_manifest_sha256,approval_registry_sha256) VALUES($1,$2,$3,'buy_and_hold','1.0.0',$4,$5,$6,'2026-08-28',$7,$8,$9,$10,$11)")
  .bind(run).bind(h.owner.user_id).bind(config).bind(strategy.config_json()).bind(strategy.config_sha256().as_str()).bind(job.id)
  .bind(daily_0060_hash_value('1')).bind(daily_0060_hash_value('2')).bind(daily_0060_hash_value('3')).bind(daily_0060_hash_value('4')).bind(daily_0060_hash_value('5'))
  .execute(&app).await?;
    app.close().await;
    Ok(run)
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
    pool: &PgPool,
    end: &str,
) -> TestResult<(serde_json::Value, serde_json::Value, serde_json::Value)> {
    use serde_json::json;
    let dates: Vec<String>=sqlx::query_scalar("SELECT (date '2026-08-29'+n)::text FROM generate_series(0,$1::text::date-date '2026-08-29') n ORDER BY n")
        .bind(end).fetch_all(pool).await?;
    if dates.is_empty() || dates.len() > 370 {
        return Err("fixture date out of bounded extension".into());
    }
    let base: Vec<String>=sqlx::query_scalar("SELECT d::text FROM (SELECT date '2016-08-29'+n d FROM generate_series(0,2450) n UNION ALL SELECT date '2026-08-28') x ORDER BY d")
        .fetch_all(pool).await?;
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
