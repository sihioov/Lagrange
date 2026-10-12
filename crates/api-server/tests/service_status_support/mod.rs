use auth::entitlement::Actor;
use chrono::NaiveDate;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const BASE_INPUT: &str = include_str!("../../../../tests/fixtures/service-status/base-input.json");
const BASE_OUTPUT: &str =
    include_str!("../../../../tests/fixtures/service-status/base-output.json");
const CASES: &str = include_str!("../../../../tests/fixtures/service-status/cases.json");
const WIRE_CASES: &str = include_str!("../../../../tests/fixtures/service-status/wire-cases.json");

#[derive(Debug)]
pub struct ProjectionCase {
    pub id: String,
    pub input: Value,
    pub expected: Value,
}

pub fn projection_cases() -> Vec<ProjectionCase> {
    let input: Value = serde_json::from_str(BASE_INPUT).expect("fixture base input parses");
    let output: Value = serde_json::from_str(BASE_OUTPUT).expect("fixture base output parses");
    let cases: Value = serde_json::from_str(CASES).expect("projection cases parse");
    let cases = cases["cases"].as_array().expect("cases array");
    let mut result = Vec::with_capacity(cases.len() + 1);
    result.push(ProjectionCase {
        id: "base".to_owned(),
        input: input.clone(),
        expected: output.clone(),
    });
    for case in cases {
        let mut case_input = input.clone();
        let mut expected = output.clone();
        apply_patches(
            &mut case_input,
            case["input_patch"].as_array().expect("input patch list"),
        );
        apply_patches(
            &mut expected,
            case["expected_patch"]
                .as_array()
                .expect("expected patch list"),
        );
        result.push(ProjectionCase {
            id: case["id"].as_str().expect("case id").to_owned(),
            input: case_input,
            expected,
        });
    }
    result
}

pub fn wire_cases() -> Value {
    serde_json::from_str(WIRE_CASES).expect("wire cases parse")
}

pub async fn required_harness() -> crate::common::Harness {
    assert!(
        std::env::var_os("DATABASE_URL").is_some(),
        "service_status requires DATABASE_URL; this suite must not silently skip"
    );
    crate::common::Harness::new()
        .await
        .expect("required service-status database harness was not created")
}

/// Create a valid current admission and exact published snapshot for the
/// requested synthetic identity range in the isolated harness database.
pub async fn seed_v2_range(
    owner_pool: &PgPool,
    owner_id: Uuid,
    first: u32,
    last: u32,
    session: NaiveDate,
    publish_snapshot: bool,
) -> Uuid {
    assert!(first > 0 && last >= first);
    let first_instrument = format!("{first:06}.KRX");
    let last_instrument = format!("{last:06}.KRX");
    let mut tx = owner_pool
        .begin()
        .await
        .expect("V2 fixture transaction begins");
    sqlx::query("SELECT pg_catalog.set_config('app.actor_user_id', $1, true)")
        .bind(owner_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("V2 fixture actor context");
    sqlx::query(
        "UPDATE public.owner_equity_universe_policies
            SET max_active_instruments=101
          WHERE owner_user_id=$1",
    )
    .bind(owner_id)
    .execute(&mut *tx)
    .await
    .expect("synthetic Owner policy supports 101st sentinel fixture");

    sqlx::query(
        "INSERT INTO public.owner_equity_memberships (
             id, owner_user_id, instrument_id, state, transition_actor_user_id,
             transition_code_commit, transition_entitlement_sha256
         )
         SELECT pg_catalog.gen_random_uuid(), $1,
                pg_catalog.lpad(sequence.number::text, 6, '0') || '.KRX',
                'REQUESTED', $1, 'c3295be', $2
           FROM pg_catalog.generate_series($3::integer, $4::integer) AS sequence(number)",
    )
    .bind(owner_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .bind(first as i32)
    .bind(last as i32)
    .execute(&mut *tx)
    .await
    .expect("synthetic Owner memberships requested");
    for state in ["VALIDATING", "BACKFILLING", "MATERIALIZING"] {
        sqlx::query(
            "UPDATE public.owner_equity_memberships
                SET state=$1, transition_actor_user_id=$2,
                    transition_code_commit='c3295be',
                    transition_entitlement_sha256=$3
              WHERE owner_user_id=$2
                AND instrument_id BETWEEN $4 AND $5",
        )
        .bind(state)
        .bind(owner_id)
        .bind(format!("sha256:{}", "e".repeat(64)))
        .bind(&first_instrument)
        .bind(&last_instrument)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|error| panic!("synthetic Owner state {state} transition failed: {error}"));
    }
    sqlx::query(
        "INSERT INTO public.owner_equity_instrument_generations (
             id, membership_id, owner_user_id, instrument_id, generation,
             target_observed_sessions, minimum_observed_sessions, observed_sessions,
             first_session, last_session
         )
         SELECT pg_catalog.gen_random_uuid(), membership.id, membership.owner_user_id,
                membership.instrument_id, 1, 121, 121, 121, $3, $3
           FROM public.owner_equity_memberships AS membership
          WHERE membership.owner_user_id=$1
            AND membership.instrument_id BETWEEN $2 AND $4",
    )
    .bind(owner_id)
    .bind(&first_instrument)
    .bind(session)
    .bind(&last_instrument)
    .execute(&mut *tx)
    .await
    .expect("synthetic Owner generations seeded");
    sqlx::query(
        "INSERT INTO public.owner_equity_generation_admissions (
             generation_id, owner_user_id, membership_id, instrument_id, generation,
             raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
             capture_code_commit, materializer_code_commit
         )
         SELECT generation.id, generation.owner_user_id, generation.membership_id,
                generation.instrument_id, generation.generation,
                $2, $2, $2, 'c3295be', 'c3295be'
           FROM public.owner_equity_instrument_generations AS generation
          WHERE generation.owner_user_id=$1
            AND generation.instrument_id BETWEEN $3 AND $4",
    )
    .bind(owner_id)
    .bind(format!("sha256:{}", "f".repeat(64)))
    .bind(&first_instrument)
    .bind(&last_instrument)
    .execute(&mut *tx)
    .await
    .expect("synthetic Owner admissions seeded");
    sqlx::query(
        "UPDATE public.owner_equity_memberships
            SET state='READY', transition_actor_user_id=$1,
                transition_code_commit='c3295be',
                transition_entitlement_sha256=$2
          WHERE owner_user_id=$1
            AND instrument_id BETWEEN $3 AND $4",
    )
    .bind(owner_id)
    .bind(format!("sha256:{}", "e".repeat(64)))
    .bind(&first_instrument)
    .bind(&last_instrument)
    .execute(&mut *tx)
    .await
    .expect("synthetic Owner memberships become READY");

    let snapshot_id = Uuid::new_v4();
    if publish_snapshot {
        let count: i32 = sqlx::query_scalar(
            "SELECT count(*)::integer
               FROM public.owner_equity_memberships
              WHERE owner_user_id=$1 AND state='READY'",
        )
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .expect("synthetic ready universe count queried");
        sqlx::query(
            "INSERT INTO public.owner_equity_signal_snapshots (
                 id, owner_user_id, as_of_session, universe_sha256, row_count,
                 signal_code_commit, published_at
             ) VALUES ($1, $2, $3, $4, $5, 'c3295be', NULL)",
        )
        .bind(snapshot_id)
        .bind(owner_id)
        .bind(session)
        .bind(sqlx::query_scalar::<_, String>(
            "SELECT 'sha256:' || pg_catalog.encode(
                 pg_catalog.sha256(pg_catalog.convert_to(
                     COALESCE(pg_catalog.string_agg(instrument_id, pg_catalog.chr(10) ORDER BY instrument_id), ''),
                     'UTF8'
                 )),
                 'hex'
             )
               FROM public.owner_equity_memberships
              WHERE owner_user_id=$1 AND state='READY'",
        )
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .expect("synthetic ready universe hash computed"))
        .bind(count)
        .execute(&mut *tx)
        .await
        .expect("synthetic V2 snapshot published");
        sqlx::query(
            "INSERT INTO public.owner_equity_signal_snapshot_rows (
                 snapshot_id, owner_user_id, instrument_id, membership_id,
                 generation_id, generation, rank, signals_json
             )
             SELECT $1, generation.owner_user_id, generation.instrument_id,
                    generation.membership_id, generation.id, generation.generation,
                    pg_catalog.row_number() OVER (ORDER BY generation.instrument_id)::integer,
                    '{}'::jsonb
               FROM public.owner_equity_instrument_generations AS generation
              JOIN public.owner_equity_memberships AS membership
                ON membership.id=generation.membership_id
               AND membership.state='READY'
              WHERE generation.owner_user_id=$2",
        )
        .bind(snapshot_id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .expect("synthetic V2 snapshot rows seeded");
        sqlx::query(
            "UPDATE public.owner_equity_signal_snapshots
                SET published_at=pg_catalog.clock_timestamp()
              WHERE id=$1 AND owner_user_id=$2",
        )
        .bind(snapshot_id)
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .expect("synthetic V2 snapshot published after its rows");
    }
    tx.commit().await.expect("V2 fixture commits");
    snapshot_id
}

pub fn v2_group(input: &Value) -> &Value {
    input
        .pointer("/db_read_results/v2")
        .expect("V2 read result present")
}

pub fn actor_session_hash(actor: &Actor, session_cookie: &str) -> String {
    assert!(!actor.user_id.0.is_empty());
    auth::sessions::cookie::hash(session_cookie)
}

pub async fn audit_rows_for_actor(
    harness: &crate::common::Harness,
    actor_id: Uuid,
) -> Vec<sqlx::postgres::PgRow> {
    sqlx::query(
        "SELECT action, target_type, target_id, reason, correlation_id
               FROM public.audit_logs
              WHERE actor_user_id=$1 AND action='admin.service_status.read'
              ORDER BY created_at, id",
    )
    .bind(actor_id)
    .fetch_all(&harness.admin_pool)
    .await
    .expect("service-status audit rows are readable")
}

fn apply_patches(document: &mut Value, patches: &[Value]) {
    for patch in patches {
        let operation = patch["op"].as_str().expect("patch op");
        let pointer = patch["path"].as_str().expect("patch path");
        match operation {
            "replace" => {
                *document
                    .pointer_mut(pointer)
                    .expect("replace target exists") = patch["value"].clone()
            }
            "add" => add(document, pointer, patch["value"].clone()),
            "remove" => remove(document, pointer),
            other => panic!("unsupported fixture patch operation {other}"),
        }
    }
}

fn tokens(pointer: &str) -> Vec<String> {
    assert!(pointer.starts_with('/'), "non-root JSON pointer expected");
    pointer[1..]
        .split('/')
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn parent_mut<'a, 'b>(document: &'a mut Value, path: &'b [String]) -> (&'a mut Value, &'b str) {
    assert!(!path.is_empty());
    let mut cursor = document;
    for token in &path[..path.len() - 1] {
        cursor = match cursor {
            Value::Object(object) => object.get_mut(token).expect("pointer object exists"),
            Value::Array(array) => array
                .get_mut(token.parse::<usize>().expect("pointer array index"))
                .expect("pointer array item exists"),
            _ => panic!("pointer parent is not a container"),
        };
    }
    (cursor, path.last().expect("pointer tail"))
}

fn add(document: &mut Value, pointer: &str, value: Value) {
    let path = tokens(pointer);
    let (parent, tail) = parent_mut(document, &path);
    match parent {
        Value::Object(object) => {
            object.insert(tail.to_owned(), value);
        }
        Value::Array(array) if tail == "-" => array.push(value),
        Value::Array(array) => {
            array.insert(tail.parse::<usize>().expect("array insertion index"), value)
        }
        _ => panic!("add target parent is not a container"),
    }
}

fn remove(document: &mut Value, pointer: &str) {
    let path = tokens(pointer);
    let (parent, tail) = parent_mut(document, &path);
    match parent {
        Value::Object(object) => {
            object.remove(tail).expect("remove object key exists");
        }
        Value::Array(array) => {
            array.remove(tail.parse::<usize>().expect("array removal index"));
        }
        _ => panic!("remove target parent is not a container"),
    }
}

/// Seed compact queue evidence through the migration-owner fixture boundary.
/// The production reader still runs as app with the actual actor RLS GUC.
pub async fn seed_queue_jobs(
    h: &crate::common::Harness,
    owner: Uuid,
    kind: &str,
    count: i32,
    status: &str,
    age_seconds: i32,
) -> Vec<Uuid> {
    let mut tx = h.owner_pool.begin().await.expect("queue seed transaction");
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(owner.to_string())
        .execute(&mut *tx)
        .await
        .expect("queue seed actor");
    let ids=sqlx::query_scalar(
        "INSERT INTO public.jobs(owner_user_id,job_type,status,available_at,locked_at,finished_at)
         SELECT $1,$2,$3,clock_timestamp()-make_interval(secs=>$5),
                CASE WHEN $3='RUNNING' THEN clock_timestamp()-make_interval(secs=>$5) END,
                CASE WHEN $3 IN ('SUCCEEDED','FAILED','CANCELED') THEN clock_timestamp()-make_interval(secs=>$5) END
           FROM generate_series(1,$4::integer) RETURNING id"
    ).bind(owner).bind(kind).bind(status).bind(count).bind(age_seconds).fetch_all(&mut *tx).await.expect("queue source fixtures seeded");
    tx.commit().await.expect("queue seed commits");
    ids
}

pub fn queue_leases() -> Value {
    json!({"owner_equity_v2":60,"owner_beta_price_recommendation":60,"candidate_compute":60,"recommendation":60,"backtest":60})
}

/// Actual immutable strategy/job bindings for historical Owner beta reads.
/// Fixture writes use migration_owner; serving reads retain actor RLS.
pub async fn seed_beta_history(
    h: &crate::common::Harness,
    count: i32,
    status: &str,
    age_seconds: i32,
) -> Vec<chrono::DateTime<chrono::Utc>> {
    assert!(matches!(status, "SUCCEEDED" | "FAILED"));
    let strategy = format!("status-beta-{}", Uuid::new_v4());
    let config = Uuid::new_v4();
    let mut tx = h.owner_pool.begin().await.expect("beta history fixture");
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(h.owner.user_id.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO strategies(id,display_name) VALUES($1,'Synthetic status history')")
        .bind(&strategy)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_strategy_configs(id,owner_user_id,strategy_id,strategy_version,config_json) VALUES($1,$2,$3,'1.0.0','{}'::jsonb)")
        .bind(config).bind(h.owner.user_id).bind(&strategy).execute(&mut *tx).await.unwrap();
    let finished = sqlx::query_scalar(r#"
        WITH fixtures AS MATERIALIZED (
          SELECT gen_random_uuid() AS run_id,gen_random_uuid() AS job_id,
                 clock_timestamp()-make_interval(secs=>$6) AS finished_at,
                 'sha256:'||repeat('a',64) AS hash FROM generate_series(1,$5::integer)
        ), jobs AS (
          INSERT INTO public.jobs(id,owner_user_id,job_type,status,available_at,finished_at,payload_json)
          SELECT job_id,$1,'owner_beta_price_recommendation',$4,finished_at,finished_at,
                 jsonb_build_object('run_id',run_id::text,'strategy_config_id',$2::uuid::text,
                   'as_of',(finished_at AT TIME ZONE 'Asia/Seoul')::date::text,
                   'pins',jsonb_build_object('candidate_content_sha256',hash,'artifact_manifest_sha256',hash,
                     'stage5_manifest_sha256',hash,'action_manifest_sha256',hash,'approval_registry_sha256',hash),
                   'strategy',jsonb_build_object('strategy_id',$3::text,'strategy_version','1.0.0',
                     'config_json','{}'::jsonb,'config_sha256',hash))
            FROM fixtures RETURNING id
        )
        INSERT INTO public.owner_beta_recommendation_runs(
          id,owner_user_id,strategy_config_id,job_id,as_of,status,
          candidate_content_sha256,artifact_manifest_sha256,stage5_manifest_sha256,
          action_manifest_sha256,approval_registry_sha256,strategy_id,strategy_version,
          strategy_config_json,strategy_config_sha256,factor_snapshot_sha256,target_snapshot_sha256,
          cash_weight,finished_at)
        SELECT fixture.run_id,$1,$2,fixture.job_id,(fixture.finished_at AT TIME ZONE 'Asia/Seoul')::date,$4,
               hash,hash,hash,hash,hash,$3,'1.0.0','{}'::jsonb,hash,
               CASE WHEN $4='SUCCEEDED' THEN hash END,CASE WHEN $4='SUCCEEDED' THEN hash END,
               CASE WHEN $4='SUCCEEDED' THEN 1::numeric END,fixture.finished_at
          FROM fixtures AS fixture JOIN jobs ON jobs.id=fixture.job_id RETURNING finished_at
        "#).bind(h.owner.user_id).bind(config).bind(strategy).bind(status).bind(count).bind(age_seconds)
        .fetch_all(&mut *tx).await.expect("real beta strategy/job binding fixtures");
    tx.commit().await.unwrap();
    finished
}

pub async fn read_queue_summary(h: &crate::common::Harness, actor: &Actor) -> Value {
    use sqlx::Row;
    let mut tx = h
        .app_pool
        .begin()
        .await
        .expect("queue app-role transaction");
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await
        .expect("queue read-only");
    api_server::actor_tx::set_actor_guc(&mut tx, actor)
        .await
        .expect("queue actual actor context");
    sqlx::query("SET LOCAL statement_timeout='250ms'")
        .execute(&mut *tx)
        .await
        .expect("queue statement budget");
    sqlx::query("SET LOCAL lock_timeout='50ms'")
        .execute(&mut *tx)
        .await
        .expect("queue lock budget");
    let row = sqlx::query(api_server::repos::service_status::QUEUE_SQL)
        .bind(h.owner.user_id)
        .bind(queue_leases())
        .fetch_one(&mut *tx)
        .await
        .expect("real bounded queue query");
    let groups: Value = row.try_get("groups").expect("queue summary groups");
    tx.commit().await.expect("queue reader commits");
    groups
}

pub async fn seed_etf_publication(
    h: &crate::common::Harness,
    covered: usize,
) -> (Uuid, Uuid, NaiveDate) {
    const ETF11: [&str; 11] = [
        "069500.KRX",
        "102110.KRX",
        "229200.KRX",
        "143850.KRX",
        "133690.KRX",
        "195930.KRX",
        "192090.KRX",
        "148070.KRX",
        "114260.KRX",
        "153130.KRX",
        "132030.KRX",
    ];
    assert!((1..=11).contains(&covered));
    let session:NaiveDate=sqlx::query_scalar("SELECT CASE WHEN (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::time<TIME '16:30' THEN (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date-1 ELSE (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date END").fetch_one(&h.owner_pool).await.expect("actual fixture eligibility date");
    let entitlement = Uuid::new_v4();
    let raw = Uuid::new_v4();
    let mut tx = h.owner_pool.begin().await.expect("ETF fixture transaction");
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(h.owner.user_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("ETF fixture actor");
    for instrument in ETF11 {
        sqlx::query("INSERT INTO instruments(id,symbol,venue,currency,name,asset_class,status) VALUES($1,$2,'KRX','KRW','Synthetic ETF','ETF','ACTIVE') ON CONFLICT(id) DO NOTHING").bind(instrument).bind(instrument.trim_end_matches(".KRX")).execute(&mut *tx).await.expect("ETF identity seeded");
    }
    sqlx::query(r#"INSERT INTO data_entitlements(id,contract_document_sha256,contract_reference,status,covered_datasets,covered_uses,effective_from,effective_until,managed_by) VALUES($1,repeat('e',64),'fixture:status-etf','ACTIVE','["krx_eod_bars"]'::jsonb,'["candidate","dataset","recommendation","backtest","paper_view"]'::jsonb,$2::date-370,$2::date+370,$3)"#)
        .bind(entitlement).bind(session).bind(h.owner.user_id).execute(&mut *tx).await.expect("exact ETF entitlement seeded");
    sqlx::query("INSERT INTO data_batches(id,source_batch_id,provider,market,batch_date,kind,storage_path,content_sha256,bytes_size,retrieved_at,source_file_name,fetch_mode) VALUES($1,$1,'KRX','KR',$2,'EOD','fixture/status-etf/bars.json',repeat('a',64),1,clock_timestamp()-interval '1 second','bars.json','credentialed')")
        .bind(raw).bind(session).execute(&mut *tx).await.expect("credentialed ETF Raw lineage seeded");
    let coverage=ETF11[..covered].iter().map(|instrument|json!({"instrument_id":instrument,"first_session":session.to_string(),"last_session":session.to_string(),"session_count":1,"sessions":[session.to_string()]})).collect::<Vec<_>>();
    let dataset:Uuid=sqlx::query_scalar(
        "SELECT dataset_version_id FROM public.publish_candidate_price_publication(
            $1,repeat('a',64),'fixture/status-etf/curated',1,$2,$2,$3::jsonb,'kis',$4,'fixture:status-etf',$5::text,$6,repeat('a',64),'credentialed',$2,clock_timestamp()-interval '2 seconds',clock_timestamp()-interval '1 second')"
    ).bind(format!("status-etf-{}",Uuid::new_v4())).bind(session).bind(json!(coverage)).bind(entitlement).bind(raw.to_string()).bind(raw).fetch_one(&mut *tx).await.expect("real guarded ETF publication");
    tx.commit().await.expect("ETF fixture commits");
    (dataset, entitlement, session)
}

pub async fn seed_calendar_days(h: &crate::common::Harness) -> NaiveDate {
    let today: NaiveDate =
        sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date")
            .fetch_one(&h.owner_pool)
            .await
            .expect("calendar fixture KST date");
    for date in [today.pred_opt().expect("previous date"), today] {
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM trading_calendars WHERE exchange='KRX' AND session_date=$1)").bind(date).fetch_one(&h.owner_pool).await.expect("existing fixture calendar");
        if exists {
            continue;
        }
        let batch = Uuid::new_v4();
        let mut tx = h
            .owner_pool
            .begin()
            .await
            .expect("calendar fixture transaction");
        sqlx::query("INSERT INTO data_batches(id,source_batch_id,provider,market,batch_date,kind,storage_path,content_sha256,bytes_size,retrieved_at,source_file_name,fetch_mode) VALUES($1,$1,'KRX','KR',$2,'CALENDAR','fixture/status-calendar/calendar.json',repeat('c',64),1,clock_timestamp()-interval '1 second','calendar.json','credentialed')").bind(batch).bind(date).execute(&mut *tx).await.expect("calendar Raw batch");
        sqlx::query("INSERT INTO trading_calendars(exchange,session_date,session_type,timezone,source,source_version,source_batch_id,content_sha256,retrieved_at) VALUES('KRX',$1,'TRADING','Asia/Seoul','kis','kis-chk-holiday-v1:schema-1',$2,repeat('c',64),clock_timestamp()-interval '1 second')").bind(date).bind(batch).execute(&mut *tx).await.expect("calendar current projection");
        sqlx::query("INSERT INTO trading_calendar_versions(exchange,session_date,session_type,timezone,source,source_version,source_batch_id,content_sha256,retrieved_at) VALUES('KRX',$1,'TRADING','Asia/Seoul','kis','kis-chk-holiday-v1:schema-1',$2,repeat('c',64),clock_timestamp()-interval '1 second')").bind(date).bind(batch).execute(&mut *tx).await.expect("calendar immutable version");
        tx.commit().await.expect("calendar fixture commits");
    }
    today
}

pub fn attach_synthetic_host(input: &mut Value, now: chrono::DateTime<chrono::Utc>) {
    fn shift(value: &mut Value, delta: chrono::Duration) {
        match value {
            Value::String(text) => {
                if let Ok(at) = chrono::DateTime::parse_from_rfc3339(text) {
                    *text = (at.with_timezone(&chrono::Utc) + delta)
                        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
                }
            }
            Value::Array(values) => {
                for value in values {
                    shift(value, delta);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    shift(value, delta);
                }
            }
            _ => {}
        }
    }
    let base: Value = serde_json::from_str(BASE_INPUT).expect("synthetic host base");
    let clock = chrono::DateTime::parse_from_rfc3339(base["clock"].as_str().expect("base clock"))
        .expect("base instant")
        .with_timezone(&chrono::Utc);
    let mut host = base["host_read"].clone();
    // The accepted wire supports at most microsecond precision. Capture one
    // synthetic clock at that precision; do not relax the production parser.
    let now = chrono::DateTime::from_timestamp_micros(now.timestamp_micros())
        .expect("synthetic host microsecond clock");
    shift(&mut host, now - clock);
    input["expected_release"] = host["snapshot"]["policy"]["release_commit"].clone();
    input["host_read"] = host;
    input["host_error"] = Value::Null;
}

// Historic source join fixture adapted from http_candidates.rs. This proves
// stored publication coverage, not present-day freshness or provider execution.
const INSTRUMENTS: [&str; 5] = [
    "200001.KRX",
    "200002.KRX",
    "200003.KRX",
    "200004.KRX",
    "200005.KRX",
];
pub async fn seed_published_candidate_feed(h: &crate::common::Harness) -> Uuid {
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO instruments
             (id, symbol, venue, currency, name, asset_class, status, listed_at)
             VALUES ($1,$2,'KRX','KRW',$3,'EQUITY','ACTIVE',DATE '2010-01-01')",
        )
        .bind(instrument)
        .bind(instrument.trim_end_matches(".KRX"))
        .bind(format!("Candidate {instrument}"))
        .execute(&h.owner_pool)
        .await
        .expect("seed candidate instrument");
    }
    let price: (Uuid, String, String) = sqlx::query_as(
        "SELECT id, version, manifest_sha256 FROM dataset_versions
         WHERE dataset_id='krx_eod_bars' AND status='READY' ORDER BY created_at LIMIT 1",
    )
    .fetch_one(&h.owner_pool)
    .await
    .expect("price dataset");
    let entitlement_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM data_entitlements WHERE status='ACTIVE' AND contract_reference='krx-2026-01' ORDER BY id LIMIT 1",
    )
    .fetch_one(&h.owner_pool)
    .await
    .expect("active candidate entitlement");
    sqlx::query(
        "UPDATE data_entitlements
            SET covered_datasets = $1,
                covered_uses = '[\"dataset\",\"factor\",\"recommendation\",\"candidate\",\"backtest\",\"report\",\"benchmark\",\"paper_view\",\"payload\",\"download\"]'::jsonb,
                updated_at = clock_timestamp()
          WHERE id = $2",
    )
    .bind(json!([
        "krx_eod_bars",
        "krx_market_status",
        "krx_investor_flows",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_kosdaq150_membership",
        "krx_sector_classification"
    ]))
    .bind(entitlement_id)
    .execute(&h.owner_pool)
    .await
    .expect("enable candidate entitlement before source inserts");
    let mut datasets = Vec::new();
    for (dataset_id, version, hash) in [
        ("krx_market_status", "candidate-http-status", "1".repeat(64)),
        ("krx_investor_flows", "candidate-http-flow", "2".repeat(64)),
        (
            "krx_fundamentals",
            "candidate-http-fundamental",
            "3".repeat(64),
        ),
        (
            "krx_kospi200_membership",
            "candidate-http-universe",
            "4".repeat(64),
        ),
        (
            "krx_sector_classification",
            "candidate-http-sector",
            "5".repeat(64),
        ),
    ] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO dataset_versions
             (dataset_id, version, status, manifest_sha256, storage_path)
             VALUES ($1,$2,'READY',$3,$4) RETURNING id",
        )
        .bind(dataset_id)
        .bind(version)
        .bind(&hash)
        .bind(format!("db://candidate-http/{dataset_id}"))
        .fetch_one(&h.owner_pool)
        .await
        .expect("seed candidate dataset");
        datasets.push((dataset_id, id, hash));
    }
    let find = |dataset_id: &str| {
        datasets
            .iter()
            .find(|row| row.0 == dataset_id)
            .expect("fixture dataset")
    };
    let status_pin = find("krx_market_status").clone();
    let flow_pin = find("krx_investor_flows").clone();
    let fundamental_pin = find("krx_fundamentals").clone();
    let universe_pin = find("krx_kospi200_membership").clone();
    let sector_pin = find("krx_sector_classification").clone();

    let mut tx = h.owner_pool.begin().await.expect("candidate seed tx");
    let raw_batch_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO data_batches
         (provider, market, batch_date, kind, storage_path, content_sha256,
          bytes_size, retrieved_at, source_batch_id, source_file_name, fetch_mode)
         VALUES ('KRX','KR',DATE '2026-08-13','EOD','raw/candidate-http',repeat('9',64),
                 1,TIMESTAMPTZ '2026-08-13 07:00:00Z',$1,'candidate-bars.json','credentialed')",
    )
    .bind(raw_batch_id)
    .execute(&mut *tx)
    .await
    .expect("seed confirmed EOD batch");
    sqlx::query(
        "INSERT INTO trading_calendars
         (exchange, session_date, session_type, timezone, source, source_version,
          source_batch_id, content_sha256, retrieved_at)
         VALUES ('KRX',DATE '2026-08-13','TRADING','Asia/Seoul','synthetic',
                 'candidate-http-calendar-1',$1,repeat('8',64),
                 TIMESTAMPTZ '2026-08-13 07:00:00Z')",
    )
    .bind(raw_batch_id)
    .execute(&mut *tx)
    .await
    .expect("seed confirmed KRX session");
    sqlx::query(
        "INSERT INTO candidate_price_publications
         (dataset_version_id, dataset_version, manifest_sha256, market,
          curated_generation, first_session, last_session, provider,
          entitlement_id, license_ref, source_revision, available_at, retrieved_at)
         VALUES ($1,$2,$3,'kr',2,DATE '2026-01-01',DATE '2026-08-13','synthetic',
                 $4,'krx-2026-01','candidate-price-1',
                 TIMESTAMPTZ '2026-08-13 06:40:00Z',TIMESTAMPTZ '2026-08-13 07:00:00Z')",
    )
    .bind(price.0)
    .bind(&price.1)
    .bind(&price.2)
    .bind(entitlement_id)
    .execute(&mut *tx)
    .await
    .expect("seed candidate price publication");
    let universe_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_universe_snapshots
         (index_id, as_of_date, dataset_version_id, manifest_sha256, provider,
          entitlement_id, entitlement_date, license_ref, source_revision,
          available_at, retrieved_at, member_count)
         VALUES ('kospi200',DATE '2026-08-13',$1,$2,'synthetic',
                 $3,DATE '2026-08-13','krx-2026-01','fixture-universe-1',
                 TIMESTAMPTZ '2026-08-13 06:40:00Z',
                 TIMESTAMPTZ '2026-08-13 07:00:00Z',5) RETURNING id",
    )
    .bind(universe_pin.1)
    .bind(&universe_pin.2)
    .bind(entitlement_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed candidate universe");
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO candidate_universe_members
             (universe_snapshot_id, instrument_id, announced_at, effective_from,
              available_at, source_revision)
             VALUES ($1,$2,TIMESTAMPTZ '2026-01-01 00:00:00Z',DATE '2026-01-01',
                     TIMESTAMPTZ '2026-01-01 00:01:00Z','fixture-universe-1')",
        )
        .bind(universe_id)
        .bind(instrument)
        .execute(&mut *tx)
        .await
        .expect("seed universe member");
    }
    let sector_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_sector_versions
          (taxonomy_id, taxonomy_version, effective_from, available_at, retrieved_at,
          provider, entitlement_id, entitlement_date, license_ref,
          source_revision, dataset_version_id, manifest_sha256)
         VALUES ('krx-sector','candidate-http-v1',DATE '2026-01-01',
                 TIMESTAMPTZ '2026-01-01 00:01:00Z',TIMESTAMPTZ '2026-08-13 07:00:00Z',
                 'synthetic',$1,DATE '2026-08-13','krx-2026-01','fixture-sector-1',$2,$3) RETURNING id",
    )
    .bind(entitlement_id)
    .bind(sector_pin.1)
    .bind(&sector_pin.2)
    .fetch_one(&mut *tx)
    .await
    .expect("seed sector version");
    for instrument in INSTRUMENTS {
        sqlx::query(
            "INSERT INTO candidate_sector_entries
             (sector_version_id, instrument_id, sector_code, sector_name,
              fundamental_profile, effective_from, available_at, source_revision)
             VALUES ($1,$2,'TECH','Technology','NON_FINANCIAL',DATE '2026-01-01',
                     TIMESTAMPTZ '2026-01-01 00:01:00Z','fixture-sector-1')",
        )
        .bind(sector_id)
        .bind(instrument)
        .execute(&mut *tx)
        .await
        .expect("seed sector entry");
        sqlx::query(
            "INSERT INTO candidate_market_status_observations
             (instrument_id, trade_date, provider, entitlement_id, entitlement_date,
              license_ref, source_revision,
              available_at, retrieved_at, dataset_version_id, manifest_sha256)
             VALUES ($1,DATE '2026-08-13','synthetic',$2,DATE '2026-08-13','krx-2026-01',
                     'fixture-status-1',TIMESTAMPTZ '2026-08-13 06:40:00Z',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z',$3,$4)",
        )
        .bind(instrument)
        .bind(entitlement_id)
        .bind(status_pin.1)
        .bind(&status_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed status observation");
        let flow_observation_id: Uuid = sqlx::query_scalar(
            "INSERT INTO candidate_investor_flows
             (instrument_id, trade_date, investor_class, net_amount, net_volume,
              provider, source_revision, available_at)
             VALUES ($1,DATE '2026-08-13','FOREIGN',1000000,100,
                     'krx','fixture-flow-1',TIMESTAMPTZ '2026-08-13 06:40:00Z')
             RETURNING id",
        )
        .bind(instrument)
        .fetch_one(&mut *tx)
        .await
        .expect("seed flow observation");
        sqlx::query(
            "INSERT INTO candidate_investor_flow_snapshot_rows
             (dataset_version_id,flow_observation_id,entitlement_id,entitlement_date,
              license_ref,retrieved_at,manifest_sha256)
             VALUES ($1,$2,$3,DATE '2026-08-13','krx-2026-01',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z',$4)",
        )
        .bind(flow_pin.1)
        .bind(flow_observation_id)
        .bind(entitlement_id)
        .bind(&flow_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed flow snapshot membership");
        sqlx::query(
            "INSERT INTO candidate_fundamental_observations
             (instrument_id, fiscal_period_start, fiscal_period_end, period_kind,
             statement_scope, metric, value, disclosed_at, available_at, retrieved_at,
              provider, entitlement_id, entitlement_date, license_ref, source_revision,
              dataset_version_id, manifest_sha256)
             VALUES ($1,DATE '2025-01-01',DATE '2025-12-31','ANNUAL','CONSOLIDATED',
                     'roe',0.15,TIMESTAMPTZ '2026-03-01 00:00:00Z',
                     TIMESTAMPTZ '2026-03-01 00:01:00Z',
                     TIMESTAMPTZ '2026-08-13 07:00:00Z','synthetic',$2,DATE '2026-08-13',
                     'krx-2026-01','fixture-fundamental-1',$3,$4)",
        )
        .bind(instrument)
        .bind(entitlement_id)
        .bind(fundamental_pin.1)
        .bind(&fundamental_pin.2)
        .execute(&mut *tx)
        .await
        .expect("seed fundamental observation");
    }

    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO stock_analysis_runs
         (as_of_date, cutoff_at, computation_seq, status, scoring_config_version,
          scoring_config_sha256, universe_key, universe_snapshot_id, universe_entitlement_id,
          price_dataset_version_id, price_entitlement_id, price_curated_version,
          price_manifest_sha256, status_dataset_version_id, status_entitlement_id,
          status_manifest_sha256, flow_dataset_version_id, flow_entitlement_id,
          flow_manifest_sha256, fundamental_dataset_version_id,
          fundamental_entitlement_id, fundamental_manifest_sha256,
          sector_version_id, sector_entitlement_id, input_identity_sha256,
          summary_json, published_at)
         VALUES (DATE '2026-08-13',TIMESTAMPTZ '2026-08-13 07:00:00Z',1,'SUCCEEDED',
                 'candidate-score-v1',
                 '1cd70f7a79af85896b015f265bea8ae931bbba29aef12a0b95f32c82ee056377',
                 'kospi200',$1,$2,$3,$4,2,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                 repeat('6',64),
                 '{\"eligible_count\":5}'::jsonb,TIMESTAMPTZ '2026-08-13 07:01:00Z')
         RETURNING id",
    )
    .bind(universe_id)
    .bind(entitlement_id)
    .bind(price.0)
    .bind(entitlement_id)
    .bind(&price.2)
    .bind(status_pin.1)
    .bind(entitlement_id)
    .bind(&status_pin.2)
    .bind(flow_pin.1)
    .bind(entitlement_id)
    .bind(&flow_pin.2)
    .bind(fundamental_pin.1)
    .bind(entitlement_id)
    .bind(&fundamental_pin.2)
    .bind(sector_id)
    .bind(entitlement_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed successful candidate run");
    let feed_id: Uuid = sqlx::query_scalar(
        "INSERT INTO candidate_feed_snapshots
         (run_id, universe_key, as_of_date, computation_seq, status, published_at)
         VALUES ($1,'kospi200',DATE '2026-08-13',1,'PUBLISHED',TIMESTAMPTZ '2026-08-13 07:01:00Z')
         RETURNING id",
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await
    .expect("seed candidate feed");
    for (index, instrument) in INSTRUMENTS.iter().enumerate() {
        let rank = i32::try_from(index + 1).expect("rank fits i32");
        let score = 90.0 - index as f64;
        let snapshot_id: Uuid = sqlx::query_scalar(
            "INSERT INTO stock_analysis_snapshots
             (run_id, instrument_id, sector_code, fundamental_profile, eligible,
              exclusion_codes, flow_score, fundamental_score, technical_score,
              total_score, flow_coverage, fundamental_coverage, technical_coverage,
              evidence_strength, rank, normalization_scope, factors_json,
              scenarios_json, provenance_json, content_sha256)
             VALUES ($1,$2,'TECH','candidate-non-financial-v1',true,'[]'::jsonb,
                     $3,$3,$3,$3,1,1,1,'STRONG',$4,'SECTOR',
                     '{\"return_20\":{\"raw\":0.1,\"normalized\":1.2}}'::jsonb,
                     '{\"bullish\":{\"label\":\"BULLISH\"},
                       \"neutral\":{\"label\":\"NEUTRAL\"},
                       \"bearish\":{\"label\":\"BEARISH\"}}'::jsonb,
                     '{\"source\":\"fixture\"}'::jsonb,repeat($5,64))
             RETURNING id",
        )
        .bind(run_id)
        .bind(instrument)
        .bind(score)
        .bind(rank)
        .bind(char::from(b'a' + u8::try_from(index).unwrap()).to_string())
        .fetch_one(&mut *tx)
        .await
        .expect("seed analysis snapshot");
        sqlx::query(
            "INSERT INTO candidate_feed_items
             (feed_id, run_id, stock_analysis_snapshot_id, instrument_id, rank)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(feed_id)
        .bind(run_id)
        .bind(snapshot_id)
        .bind(instrument)
        .bind(rank)
        .execute(&mut *tx)
        .await
        .expect("seed feed item");
    }
    tx.commit().await.expect("candidate fixture commits");

    sqlx::query(
        "UPDATE data_entitlements
            SET covered_datasets=$1, covered_uses=$2, updated_at=clock_timestamp()
          WHERE status='ACTIVE'",
    )
    .bind(json!([
        "krx_eod_bars",
        "krx_market_status",
        "krx_investor_flows",
        "krx_fundamentals",
        "krx_kospi200_membership",
        "krx_kosdaq150_membership",
        "krx_sector_classification"
    ]))
    .bind(json!([
        "dataset",
        "factor",
        "recommendation",
        "candidate",
        "backtest",
        "report",
        "benchmark",
        "paper_view",
        "payload",
        "download"
    ]))
    .execute(&h.owner_pool)
    .await
    .expect("enable candidate entitlement");
    run_id
}

pub async fn configure_stream_window(
    h: &crate::common::Harness,
    stream: &mut crate::service_status_reads_support::StreamFixture,
    closed: bool,
) -> api_server::http::state::ApiState {
    use api_server::http::owner_market_stream_config::{
        MarketStreamReadConfig, MarketStreamReadPins,
    };
    use collectors::intraday_quotes::IntradaySessionWindowContract;
    use sha2::{Digest, Sha256};
    let evidence: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT clock_timestamp()-interval '1 second'")
            .fetch_one(&h.owner_pool)
            .await
            .expect("actual window observation");
    let bytes=serde_json::to_vec(&json!({"schema_version":1,"exchange":"KRX","timezone":"Asia/Seoul","entries":[{
        "date":stream.session_date.to_string(),"disposition":if closed {"CLOSED"} else {"SPECIAL"},
        "open_local":if closed {None} else {Some("00:00:00")},"close_local":if closed {None} else {Some("23:59:59")},
        "evidence_url":"https://global.krx.co.kr/fixture/status","evidence_retrieved_at":evidence.to_rfc3339_opts(chrono::SecondsFormat::AutoSi,true),"evidence_sha256":format!("sha256:{}","f".repeat(64))
    }]})).expect("synthetic hash-pinned operational window");
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    let activation = serde_json::to_vec(&json!({"schema_version":1,"window_sha256":hash}))
        .expect("synthetic activation descriptor");
    // Match the exact operational descriptor fields; use the existing parser,
    // not an invented entry or replacement bounds in the status adapter.
    let window = IntradaySessionWindowContract::from_operational_bytes(&activation, &bytes)
        .expect("validated operational window bytes");
    stream.window_sha256 = window.window_contract_sha256().to_owned();
    let proof = format!(
        "owner-market-stream-day-v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
        stream.session_date,
        stream.calendar_batch_id,
        stream.calendar_content_sha256,
        stream.window_sha256,
        stream.open_at.format("%Y-%m-%dT%H:%M:%SZ"),
        stream.close_at.format("%Y-%m-%dT%H:%M:%SZ")
    );
    stream.session_proof_sha256 = format!("sha256:{:x}", Sha256::digest(proof.as_bytes()));
    sqlx::query("UPDATE public.owner_market_stream_producers SET window_contract_sha256=$1,session_proof_sha256=$2,heartbeat_at=clock_timestamp(),lease_expires_at=clock_timestamp()+interval '2 minutes' WHERE credential_slot_id=$3")
        .bind(&stream.window_sha256).bind(&stream.session_proof_sha256).bind(stream.slot_id).execute(&h.owner_pool).await.expect("actual synthetic producer proof pin");
    let pins = MarketStreamReadPins::new(
        "https://status.example".to_owned(),
        stream.slot_id,
        stream.grant_id,
        stream.contract_sha256.clone(),
        Some(std::sync::Arc::new(window)),
    )
    .expect("nonsecret actual reader pins");
    let mut state = h.state();
    let mut cfg = (*state.cfg).clone();
    cfg.owner_market_stream = MarketStreamReadConfig::OwnerOnly(pins);
    state.cfg = std::sync::Arc::new(cfg);
    state
}

pub async fn seed_second_stream_identity(
    h: &crate::common::Harness,
    stream: &crate::service_status_reads_support::StreamFixture,
    first: &crate::service_status_reads_support::DemandFixture,
) {
    let second = crate::service_status_reads_support::seed_second_demand_fixture(h, stream, first)
        .await
        .expect("second authenticated current demand");
    crate::service_status_reads_support::seed_live_quote_fixture(h, stream, &second)
        .await
        .expect("second authentic subscription and quote evidence");
}

pub async fn refresh_stream_clocks(
    h: &crate::common::Harness,
    stream: &crate::service_status_reads_support::StreamFixture,
) {
    let mut tx = h
        .owner_pool
        .begin()
        .await
        .expect("fresh synthetic stream observations");
    sqlx::query("SELECT set_config('app.actor_user_id',$1,true)")
        .bind(stream.owner_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("fixture refresh actor");
    sqlx::query("UPDATE owner_market_stream_producers SET heartbeat_at=clock_timestamp(),lease_expires_at=clock_timestamp()+interval '2 minutes' WHERE credential_slot_id=$1").bind(stream.slot_id).execute(&mut *tx).await.expect("producer clock refresh");
    sqlx::query("WITH capture AS MATERIALIZED(SELECT clock_timestamp()-interval '1 second' AS at) UPDATE owner_market_stream_cache SET received_at=capture.at,provider_trade_at=capture.at,committed_at=capture.at+interval '100 milliseconds',business_date=(capture.at AT TIME ZONE 'Asia/Seoul')::date,trade_time=(capture.at AT TIME ZONE 'Asia/Seoul')::time FROM capture WHERE owner_user_id=$1").bind(stream.owner_id).execute(&mut *tx).await.expect("original synthetic quote clocks");
    tx.commit().await.expect("clock fixture commits");
}
