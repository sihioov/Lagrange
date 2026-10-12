//! Bounded read-only evidence queries for the Owner service-status endpoint.
//! Each evidence group has its own READ ONLY transaction so one failed query
//! cannot poison another group's result.

use crate::actor_tx::{actor_uuid, set_actor_guc};
use crate::http::owner_market_stream_config::MarketStreamReadConfig;
use crate::http::state::{ApiState, OwnerIntradayQuoteReadConfig};
use crate::service_status::HostReadError;
use auth::entitlement::Actor;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::time::timeout;

const GROUP_TIMEOUT: Duration = Duration::from_millis(250);
const EVIDENCE_BUDGET: Duration = Duration::from_secs(2);
/// One bounded selection and two indexed current-status history lookups per
/// type. Only compact summaries leave SQL; a sentinel in one type cannot hide
/// another. Recheck boundaries prevent cached scalar counts outliving changes.
#[doc(hidden)]
pub const QUEUE_SQL: &str = r#"
WITH observation AS MATERIALIZED (SELECT clock_timestamp() AS at),
     kinds(job_type) AS (VALUES ('owner_equity_v2'),('owner_beta_price_recommendation'),
                               ('candidate_compute'),('recommendation'),('backtest')),
     summaries AS (
       SELECT kinds.job_type, observation.at, counts.*, success.finished_at AS last_success_at,
              failure.finished_at AS last_failure_at,
              ($2::jsonb->>kinds.job_type)::integer AS lease_seconds
         FROM kinds CROSS JOIN observation
         CROSS JOIN LATERAL (
           SELECT count(*) AS selected_count,
                  count(*) FILTER (WHERE job.status='QUEUED') AS queued,
                  count(*) FILTER (WHERE job.status='RUNNING') AS running,
                  count(*) FILTER (WHERE job.status='QUEUED' AND job.available_at<=observation.at) AS eligible_queued,
                  count(*) FILTER (WHERE job.status='FAILED' AND job.finished_at BETWEEN observation.at-INTERVAL '24 hours' AND observation.at) AS recent_failed,
                  count(*) FILTER (WHERE job.status='RUNNING' AND job.locked_at+make_interval(secs=>($2::jsonb->>kinds.job_type)::integer)<=observation.at) AS expired_claims,
                  min(job.available_at) FILTER (WHERE job.status='QUEUED' AND job.available_at<=observation.at) AS oldest_eligible_at,
                  min(job.available_at) FILTER (WHERE job.status='QUEUED' AND job.available_at>observation.at) AS next_available_at,
                  min(job.locked_at+make_interval(secs=>($2::jsonb->>kinds.job_type)::integer)) FILTER (WHERE job.status='RUNNING' AND job.locked_at+make_interval(secs=>($2::jsonb->>kinds.job_type)::integer)>observation.at) AS next_claim_expiry_at,
                  min(job.finished_at+INTERVAL '24 hours') FILTER (WHERE job.status='FAILED' AND job.finished_at BETWEEN observation.at-INTERVAL '24 hours' AND observation.at) AS next_failure_expiry_at,
                  COALESCE(bool_or(job.available_at IS NULL OR job.locked_at>observation.at OR job.finished_at>observation.at OR (job.status='RUNNING' AND job.locked_at IS NULL)),false) AS invalid
             FROM (
               SELECT status,available_at,locked_at,finished_at
                 FROM public.jobs
                WHERE owner_user_id=$1 AND job_type=kinds.job_type
                  AND (status IN ('QUEUED','RUNNING') OR finished_at>=observation.at-INTERVAL '24 hours')
                LIMIT 10001
             ) AS job
         ) AS counts
         LEFT JOIN LATERAL (
           SELECT finished_at FROM public.jobs
            WHERE owner_user_id=$1 AND job_type=kinds.job_type
              AND status='SUCCEEDED' AND finished_at IS NOT NULL
            ORDER BY finished_at DESC LIMIT 1
         ) AS success ON true
         LEFT JOIN LATERAL (
           SELECT finished_at FROM public.jobs
            WHERE owner_user_id=$1 AND job_type=kinds.job_type
              AND status='FAILED' AND finished_at IS NOT NULL
            ORDER BY finished_at DESC LIMIT 1
         ) AS failure ON true
     )
SELECT observation.at AS observed_at,
       (SELECT jsonb_object_agg(summary.job_type,
          (to_jsonb(summary)-'job_type'-'at'-'selected_count') || jsonb_build_object(
            'result',CASE WHEN selected_count>=10001 THEN 'limit' ELSE 'ok' END,
            'observed_at',summary.at,
            'recheck_at',least(next_available_at,next_claim_expiry_at,next_failure_expiry_at)
          )) FROM summaries AS summary) AS groups,
       (SELECT run.finished_at FROM public.owner_beta_recommendation_runs AS run
         WHERE run.owner_user_id=$1 AND run.status='SUCCEEDED' AND run.finished_at IS NOT NULL
         ORDER BY run.finished_at DESC LIMIT 1) AS owner_beta_success_at
  FROM observation
"#;

const ETF_SQL: &str = r#"
WITH required(instrument_id) AS (
    VALUES ('069500.KRX'),('102110.KRX'),('229200.KRX'),('143850.KRX'),
           ('133690.KRX'),('195930.KRX'),('192090.KRX'),('148070.KRX'),
           ('114260.KRX'),('153130.KRX'),('132030.KRX')
), raw_latest AS (
    SELECT batch.batch_date, batch.retrieved_at
      FROM public.data_batches AS batch
     WHERE batch.provider='KRX' AND batch.market='KR' AND batch.kind='EOD'
       AND batch.fetch_mode='credentialed'
       AND batch.source_batch_id IS NOT NULL AND batch.source_file_name IS NOT NULL
     ORDER BY batch.batch_date DESC, batch.retrieved_at DESC, batch.id DESC
     LIMIT 1
), publication AS (
    SELECT price.dataset_version_id, price.dataset_version, price.manifest_sha256,
           price.first_session, price.last_session, price.entitlement_id,
           price.license_ref, price.available_at, price.retrieved_at,
           dataset.status AS dataset_state,
           raw.state AS binding_state, raw.fetch_mode,
           raw.published_at AS raw_published_at,
           (SELECT count(DISTINCT session.instrument_id)
              FROM public.candidate_price_instrument_sessions AS session
              JOIN required AS need ON need.instrument_id=session.instrument_id
             WHERE session.dataset_version_id=price.dataset_version_id
               AND session.session_date=price.last_session) AS covered_count
      FROM public.candidate_price_publications AS price
      JOIN public.dataset_versions AS dataset
        ON dataset.id=price.dataset_version_id
       AND dataset.dataset_id='krx_eod_bars'
       AND dataset.version=price.dataset_version
       AND dataset.manifest_sha256=price.manifest_sha256
       AND dataset.status IN ('READY','WARNING','BLOCKED')
      JOIN public.candidate_raw_batch_datasets AS binding
        ON binding.dataset_version_id=price.dataset_version_id
       AND binding.dataset_id='krx_eod_bars'
       AND binding.surface='price' AND binding.response_kind='bars'
      JOIN public.candidate_raw_batch_publications AS raw
        ON raw.batch_id=binding.batch_id AND raw.surface='price'
       AND raw.state IN ('PUBLISHED','BLOCKED')
       AND price.source_revision=raw.batch_id::text
       AND price.license_ref=raw.entitlement_reference
       AND price.last_session=raw.entitlement_date
     WHERE price.market='kr'
     ORDER BY price.last_session DESC, raw.published_at DESC, price.available_at DESC
     LIMIT 1
)
SELECT clock_timestamp() AS observed_at,
       (SELECT batch_date FROM raw_latest) AS raw_as_of,
       (SELECT last_session FROM publication) AS publication_session,
       (SELECT raw_published_at FROM publication) AS published_at,
       (SELECT dataset_state FROM publication) AS dataset_state,
       (SELECT binding_state FROM publication) AS binding_state,
       (SELECT fetch_mode FROM publication) AS fetch_mode,
       11::bigint AS required_count,
       (SELECT covered_count FROM publication) AS covered_count,
       (SELECT jsonb_build_object(
            'entitlement_id', entitlement_id::text,
            'contract_reference', license_ref,
            'first_session', first_session::text,
            'last_session', last_session::text
        ) FROM publication) AS rights_binding
"#;

const RIGHTS_SQL: &str = r#"
SELECT clock_timestamp() AS observed_at,
       COALESCE(EXISTS (
           SELECT 1
             FROM public.data_entitlements AS entitlement
            WHERE entitlement.id=$1::uuid
              AND entitlement.contract_reference=$2
              AND entitlement.status='ACTIVE'
              AND entitlement.covered_datasets @> '["krx_eod_bars"]'::jsonb
              AND entitlement.covered_uses @> '["dataset","recommendation","backtest","paper_view"]'::jsonb
              AND entitlement.effective_from <= $3::date
              AND entitlement.effective_until >= $4::date
       ), false) AS rights_valid
"#;

#[doc(hidden)]
pub const V2_SQL: &str = r#"
WITH ready AS MATERIALIZED (
    SELECT membership.id AS membership_id, membership.instrument_id,
           generation.id AS generation_id, generation.generation,
           generation.last_session,
           admission.generation_id AS admitted_generation_id
      FROM public.owner_equity_memberships AS membership
      LEFT JOIN LATERAL (
          SELECT item.id, item.generation, item.last_session
            FROM public.owner_equity_instrument_generations AS item
           WHERE item.membership_id=membership.id
             AND item.owner_user_id=membership.owner_user_id
             AND item.instrument_id=membership.instrument_id
           ORDER BY item.generation DESC LIMIT 1
      ) AS generation ON true
      LEFT JOIN public.owner_equity_generation_admissions AS admission
        ON admission.generation_id=generation.id
       AND admission.owner_user_id=membership.owner_user_id
       AND admission.membership_id=membership.id
       AND admission.instrument_id=membership.instrument_id
       AND admission.generation=generation.generation
     WHERE membership.owner_user_id=$1 AND membership.state='READY'
     ORDER BY membership.id LIMIT 101
), latest_snapshot AS (
    SELECT snapshot.id, snapshot.as_of_session, snapshot.published_at,
           snapshot.row_count
      FROM public.owner_equity_signal_snapshots AS snapshot
     WHERE snapshot.owner_user_id=$1 AND snapshot.published_at IS NOT NULL
     ORDER BY snapshot.as_of_session DESC, snapshot.published_at DESC, snapshot.id DESC
     LIMIT 1
), snapshot_cover AS (
    SELECT count(DISTINCT signal.instrument_id) AS covered_count,
           count(*) AS row_count
      FROM latest_snapshot AS snapshot
      JOIN public.owner_equity_signal_snapshot_rows AS signal
        ON signal.snapshot_id=snapshot.id AND signal.owner_user_id=$1
      JOIN ready ON ready.membership_id=signal.membership_id
       AND ready.instrument_id=signal.instrument_id
       AND ready.generation_id=signal.generation_id
       AND ready.generation=signal.generation
       AND ready.admitted_generation_id=signal.generation_id
)
SELECT clock_timestamp() AS observed_at,
       (SELECT count(*) FROM ready) AS required_count,
       (SELECT count(*) FROM ready WHERE admitted_generation_id=generation_id) AS covered_count,
       (SELECT min(last_session) FROM ready WHERE admitted_generation_id=generation_id) AS generation_last_session,
       CASE WHEN (SELECT count(*) FROM ready) <= 100
                  AND (SELECT count(*) FROM ready WHERE admitted_generation_id=generation_id)=(SELECT count(*) FROM ready)
                  AND COALESCE((SELECT covered_count FROM snapshot_cover),0)=(SELECT count(*) FROM ready)
                  AND COALESCE((SELECT row_count FROM snapshot_cover),0)=(SELECT count(*) FROM ready)
                  AND (SELECT row_count FROM latest_snapshot)=(SELECT count(*) FROM ready)
            THEN (SELECT as_of_session FROM latest_snapshot) END AS snapshot_as_of,
       CASE WHEN (SELECT count(*) FROM ready) <= 100
                  AND (SELECT count(*) FROM ready WHERE admitted_generation_id=generation_id)=(SELECT count(*) FROM ready)
                  AND COALESCE((SELECT covered_count FROM snapshot_cover),0)=(SELECT count(*) FROM ready)
                  AND COALESCE((SELECT row_count FROM snapshot_cover),0)=(SELECT count(*) FROM ready)
                  AND (SELECT row_count FROM latest_snapshot)=(SELECT count(*) FROM ready)
            THEN (SELECT published_at FROM latest_snapshot) END AS snapshot_published_at,
       ((SELECT count(*) FROM ready) <= 100
          AND (SELECT count(*) FROM ready WHERE admitted_generation_id=generation_id)=(SELECT count(*) FROM ready)) AS current_admissions_match
"#;

const CANDIDATE_SQL: &str = r#"
WITH required AS MATERIALIZED (
    SELECT registry.universe_key
      FROM public.candidate_universe_registry AS registry
     WHERE registry.enabled
     ORDER BY registry.sort_order LIMIT 17
), latest AS MATERIALIZED (
    SELECT DISTINCT ON (required.universe_key)
           required.universe_key, feed.as_of_date, feed.published_at
      FROM required
      JOIN public.candidate_universe_snapshots AS universe
        ON universe.index_id=required.universe_key
      JOIN public.candidate_feed_snapshots AS feed
        ON feed.status='PUBLISHED'
       AND feed.as_of_date=universe.as_of_date
      JOIN public.stock_analysis_runs AS run
        ON run.id=feed.run_id AND run.universe_snapshot_id=universe.id
       AND run.status='SUCCEEDED' AND run.published_at IS NOT NULL
     ORDER BY required.universe_key, feed.as_of_date DESC,
              feed.computation_seq DESC, feed.published_at DESC
)
SELECT clock_timestamp() AS observed_at,
       COALESCE((SELECT control.active FROM public.candidate_scheduler_control AS control WHERE control.control_key='scheduler'),false) AS enabled,
       (SELECT count(*) FROM required) AS required_universes,
       (SELECT count(*) FROM latest) AS published_universes,
       (SELECT min(as_of_date) FROM latest) AS as_of,
       (SELECT min(published_at) FROM latest) AS published_at
"#;

const RESEARCH_SQL: &str = r#"
SELECT clock_timestamp() AS observed_at,
       status.attempt_started_at, status.heartbeat_at,
       status.attempt_finished_at, status.attempt_state,
       status.last_success_at, status.last_failure_at,
       status.failure_code, status.target_session, status.published_session
  FROM (SELECT 1) AS singleton
  LEFT JOIN public.research_execution_status AS status ON status.id='kis-eod'
"#;

const CALENDAR_SQL: &str = r#"
SELECT clock_timestamp() AS observed_at,
       COALESCE((
          SELECT jsonb_agg(jsonb_build_object(
              'session_date', calendar.session_date::text,
              'session_type', calendar.session_type,
              'exchange', calendar.exchange,
              'timezone', calendar.timezone,
              'source', calendar.source,
              'source_version', calendar.source_version,
              'calendar_batch', calendar.source_batch_id::text,
              'calendar_hash', calendar.content_sha256,
              'calendar_retrieved_at', to_char(calendar.retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
              'version_batch', version.source_batch_id::text,
              'version_hash', version.content_sha256,
              'version_retrieved_at', to_char(version.retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
              'raw_batch', batch.source_batch_id::text,
              'raw_hash', batch.content_sha256,
              'raw_kind', batch.kind,
              'raw_file', batch.source_file_name,
              'raw_fetch_mode', batch.fetch_mode,
              'raw_retrieved_at', to_char(batch.retrieved_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
          ) ORDER BY calendar.session_date)
            FROM (
                SELECT * FROM public.trading_calendars AS item
                 WHERE item.exchange='KRX'
                   AND item.session_date BETWEEN ((clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date - 369)
                                             AND (clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date
                 ORDER BY item.session_date DESC LIMIT 371
            ) AS calendar
            JOIN public.trading_calendar_versions AS version
              ON version.exchange=calendar.exchange
             AND version.session_date=calendar.session_date
             AND version.session_type=calendar.session_type
             AND version.timezone=calendar.timezone
             AND version.source=calendar.source
             AND version.source_version=calendar.source_version
             AND version.source_batch_id=calendar.source_batch_id
             AND version.content_sha256=calendar.content_sha256
            JOIN public.data_batches AS batch
              ON batch.source_batch_id=calendar.source_batch_id
             AND batch.provider='KRX' AND batch.market='KR'
             AND batch.batch_date=calendar.session_date
             AND batch.content_sha256=calendar.content_sha256
             AND batch.kind='CALENDAR'
             AND batch.source_file_name='calendar.json'
             AND batch.fetch_mode='credentialed'
       ), '[]'::jsonb) AS rows
"#;

const STREAM_SQL: &str = r#"
SELECT clock_timestamp() AS query_at,
       (SELECT to_jsonb(result)
          FROM public.owner_service_status_stream(
             $1::uuid,$2::text,$3::uuid,$4::uuid,$5::text,$6::text,$7::timestamptz,$8::timestamptz
          ) AS result) AS aggregate
"#;

#[derive(Debug, Clone, Copy)]
enum ReadFailure {
    Timeout,
    Unavailable,
    Denied,
    Invalid,
}

impl ReadFailure {
    fn result(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::Denied => "denied",
            Self::Invalid => "invalid",
        }
    }
}

fn classify(error: sqlx::Error) -> ReadFailure {
    if let Some(database) = error.as_database_error() {
        match database.code().as_deref() {
            Some("57014" | "55P03" | "57000") => ReadFailure::Timeout,
            Some("42501") => ReadFailure::Denied,
            Some("42P01" | "42703" | "42883" | "22P02") => ReadFailure::Invalid,
            _ => ReadFailure::Unavailable,
        }
    } else {
        match error {
            sqlx::Error::ColumnNotFound(_)
            | sqlx::Error::ColumnDecode { .. }
            | sqlx::Error::Decode(_) => ReadFailure::Invalid,
            _ => ReadFailure::Unavailable,
        }
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    DateTime::from_timestamp_micros(value.timestamp_micros())
        .expect("supported UTC source observation")
        .to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn group_failure(result: &str) -> Value {
    json!({"result": result, "observed_at": null})
}

async fn begin_read(
    pool: &PgPool,
    actor: Option<&Actor>,
    session_hash: Option<&str>,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    if let Some(actor) = actor {
        set_actor_guc(&mut tx, actor)
            .await
            .map_err(|error| match error {
                crate::error::TenancyError::Database(error) => error,
                _ => sqlx::Error::Protocol("actor identity unavailable".to_owned()),
            })?;
    }
    if let Some(session_hash) = session_hash {
        sqlx::query("SELECT set_config('app.market_stream_session_hash', $1, true)")
            .bind(session_hash)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("SET LOCAL statement_timeout = '250ms'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '50ms'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

async fn bounded<T>(
    start: Instant,
    future: impl Future<Output = Result<T, sqlx::Error>>,
) -> Result<T, ReadFailure> {
    let remaining = EVIDENCE_BUDGET.saturating_sub(start.elapsed());
    if remaining.is_zero() {
        return Err(ReadFailure::Timeout);
    }
    let budget = GROUP_TIMEOUT.min(remaining);
    timeout(budget, future)
        .await
        .map_err(|_| ReadFailure::Timeout)?
        .map_err(classify)
}

async fn read_queue(
    pool: &PgPool,
    actor: &Actor,
    owner: uuid::Uuid,
    start: Instant,
    leases: Option<&Value>,
) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, Some(actor), None).await?;
        let row = sqlx::query(QUEUE_SQL)
            .bind(owner)
            .bind(leases.cloned().unwrap_or_else(|| json!({})))
            .fetch_one(&mut *tx)
            .await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let mut groups: Value = row.try_get("groups")?;
        let owner_beta_success: Option<DateTime<Utc>> = row.try_get("owner_beta_success_at")?;
        tx.commit().await?;
        let Some(groups_object) = groups.as_object_mut() else {
            return Err(sqlx::Error::Decode("invalid queue summary".into()));
        };
        for group in groups_object.values_mut() {
            for field in [
                "observed_at",
                "oldest_eligible_at",
                "next_available_at",
                "next_claim_expiry_at",
                "next_failure_expiry_at",
                "last_success_at",
                "last_failure_at",
                "recheck_at",
            ] {
                if let Some(value) = group.get(field).and_then(Value::as_str) {
                    let parsed = DateTime::parse_from_rfc3339(value)
                        .map_err(|_| sqlx::Error::Decode("invalid queue source clock".into()))?;
                    group[field] = json!(timestamp(parsed.with_timezone(&Utc)));
                }
            }
        }
        Ok(json!({
            "result":"ok", "observed_at":timestamp(observed), "groups":groups,
            "owner_beta_success_at":owner_beta_success.map(timestamp)
        }))
    })
    .await
}

async fn read_etf(pool: &PgPool, start: Instant) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, None, None).await?;
        let row = sqlx::query(ETF_SQL).fetch_one(&mut *tx).await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let raw_as_of: Option<chrono::NaiveDate> = row.try_get("raw_as_of")?;
        let publication_session: Option<chrono::NaiveDate> = row.try_get("publication_session")?;
        let published_at: Option<DateTime<Utc>> = row.try_get("published_at")?;
        let dataset_state: Option<String> = row.try_get("dataset_state")?;
        let binding_state: Option<String> = row.try_get("binding_state")?;
        let fetch_mode: Option<String> = row.try_get("fetch_mode")?;
        let required_count: i64 = row.try_get("required_count")?;
        let covered_count: Option<i64> = row.try_get("covered_count")?;
        let rights_binding: Option<Value> = row.try_get("rights_binding")?;
        tx.commit().await?;
        Ok(json!({
            "result":"ok", "observed_at":timestamp(observed),
            "raw_as_of":raw_as_of.map(|date| date.to_string()),
            "publication_session":publication_session.map(|date| date.to_string()),
            "published_at":published_at.map(timestamp),
            "dataset_state":dataset_state, "binding_state":binding_state,
            "fetch_mode":fetch_mode, "required_count":required_count,
            "covered_count":covered_count, "rights_binding":rights_binding
        }))
    })
    .await
}

async fn read_rights(pool: &PgPool, etf: &Value, start: Instant) -> Result<Value, ReadFailure> {
    let binding = etf.get("rights_binding").filter(|value| !value.is_null());
    bounded(start, async {
        let mut tx = begin_read(pool, None, None).await?;
        let row = if let Some(binding) = binding {
            sqlx::query(RIGHTS_SQL)
                .bind(binding.get("entitlement_id").and_then(Value::as_str))
                .bind(binding.get("contract_reference").and_then(Value::as_str))
                .bind(binding.get("first_session").and_then(Value::as_str))
                .bind(binding.get("last_session").and_then(Value::as_str))
                .fetch_one(&mut *tx)
                .await?
        } else {
            sqlx::query("SELECT clock_timestamp() AS observed_at, false AS rights_valid")
                .fetch_one(&mut *tx)
                .await?
        };
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let rights_valid: bool = row.try_get("rights_valid")?;
        tx.commit().await?;
        Ok(json!({"result":"ok","observed_at":timestamp(observed),"rights_valid":rights_valid}))
    })
    .await
}

async fn read_v2(
    pool: &PgPool,
    actor: &Actor,
    owner: uuid::Uuid,
    start: Instant,
) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, Some(actor), None).await?;
        let row = sqlx::query(V2_SQL).bind(owner).fetch_one(&mut *tx).await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let required: i64 = row.try_get("required_count")?;
        let covered: i64 = row.try_get("covered_count")?;
        let generation_last: Option<chrono::NaiveDate> = row.try_get("generation_last_session")?;
        let snapshot_as_of: Option<chrono::NaiveDate> = row.try_get("snapshot_as_of")?;
        let snapshot_published_at: Option<DateTime<Utc>> = row.try_get("snapshot_published_at")?;
        let current_admissions_match: bool = row.try_get("current_admissions_match")?;
        tx.commit().await?;
        Ok(json!({
            "result":"ok","observed_at":timestamp(observed),
            "required_count":required,"covered_count":covered,
            "generation_last_session":generation_last.map(|date|date.to_string()),
            "snapshot_as_of":snapshot_as_of.map(|date|date.to_string()),
            "snapshot_published_at":snapshot_published_at.map(timestamp),
            "current_admissions_match":current_admissions_match
        }))
    })
    .await
}

async fn read_candidate(pool: &PgPool, start: Instant) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, None, None).await?;
        let row = sqlx::query(CANDIDATE_SQL).fetch_one(&mut *tx).await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let enabled: bool = row.try_get("enabled")?;
        let required: i64 = row.try_get("required_universes")?;
        let published: i64 = row.try_get("published_universes")?;
        let as_of: Option<chrono::NaiveDate> = row.try_get("as_of")?;
        let published_at: Option<DateTime<Utc>> = row.try_get("published_at")?;
        tx.commit().await?;
        Ok(
            json!({"result":"ok","observed_at":timestamp(observed),"enabled":enabled,
            "required_universes":required,"published_universes":published,
            "as_of":as_of.map(|date|date.to_string()),"published_at":published_at.map(timestamp)}),
        )
    })
    .await
}

async fn read_research(pool: &PgPool, start: Instant) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, None, None).await?;
        let row = sqlx::query(RESEARCH_SQL).fetch_one(&mut *tx).await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let started: Option<DateTime<Utc>> = row.try_get("attempt_started_at")?;
        let heartbeat: Option<DateTime<Utc>> = row.try_get("heartbeat_at")?;
        let finished: Option<DateTime<Utc>> = row.try_get("attempt_finished_at")?;
        let attempt_state: Option<String> = row.try_get("attempt_state")?;
        let last_success: Option<DateTime<Utc>> = row.try_get("last_success_at")?;
        let last_failure: Option<DateTime<Utc>> = row.try_get("last_failure_at")?;
        let failure_code: Option<String> = row.try_get("failure_code")?;
        let target_session: Option<chrono::NaiveDate> = row.try_get("target_session")?;
        let published_session: Option<chrono::NaiveDate> = row.try_get("published_session")?;
        tx.commit().await?;
        let row = if let (Some(started), Some(heartbeat), Some(attempt_state)) = (started, heartbeat, attempt_state) {
            Some(json!({
                "attempt_started_at":timestamp(started),"heartbeat_at":timestamp(heartbeat),
                "attempt_finished_at":finished.map(timestamp),"attempt_state":attempt_state,
                "last_success_at":last_success.map(timestamp),"last_failure_at":last_failure.map(timestamp),
                "failure_code":failure_code,"target_session":target_session.map(|date|date.to_string()),
                "published_session":published_session.map(|date|date.to_string())
            }))
        } else { None };
        Ok(json!({"result":"ok","observed_at":timestamp(observed),"row":row}))
    }).await
}

async fn read_calendar(pool: &PgPool, actor: &Actor, start: Instant) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let mut tx = begin_read(pool, Some(actor), None).await?;
        let row = sqlx::query(CALENDAR_SQL).fetch_one(&mut *tx).await?;
        let observed: DateTime<Utc> = row.try_get("observed_at")?;
        let rows: Value = row.try_get("rows")?;
        tx.commit().await?;
        let limited = serde_json::to_vec(&rows).map_or(true, |bytes| bytes.len() > 256 * 1024);
        Ok(json!({"result":if limited {"limit"} else {"ok"},
            "observed_at":timestamp(observed),"rows":if limited {json!([])} else {rows}}))
    })
    .await
}

async fn read_stream(
    pool: &PgPool,
    actor: &Actor,
    owner: uuid::Uuid,
    session_hash: &str,
    state: &ApiState,
    now: DateTime<Utc>,
    start: Instant,
) -> Result<Value, ReadFailure> {
    bounded(start, async {
        let (slot, grant, contract, window_sha, open, close) = stream_pins(state, now);
        let mut tx = begin_read(pool, Some(actor), Some(session_hash)).await?;
        let row = sqlx::query(STREAM_SQL)
            .bind(owner)
            .bind(session_hash)
            .bind(slot)
            .bind(grant)
            .bind(contract)
            .bind(window_sha)
            .bind(open)
            .bind(close)
            .fetch_one(&mut *tx)
            .await?;
        let query_at: DateTime<Utc> = row.try_get("query_at")?;
        let aggregate: Option<Value> = row.try_get("aggregate")?;
        tx.commit().await?;
        Ok(adapt_stream_aggregate(aggregate, query_at))
    })
    .await
}

// SQL counts are inclusive coverage; fixture inputs are already exclusive.
// Conversion exists only at this real repository seam, exactly once.
fn adapt_stream_aggregate(aggregate: Option<Value>, query_at: DateTime<Utc>) -> Value {
    let Some(mut aggregate) = aggregate else {
        return json!({"result":"ok","observed_at":timestamp(query_at),"owner_binding_valid":false,
                "demand_count":null,"live_count":null,"halted_count":null});
    };
    const FIELDS: [&str; 16] = [
        "observed_at",
        "demand_count",
        "live_count",
        "halted_count",
        "producer_present",
        "connection_state",
        "status_code",
        "producer_status_at",
        "producer_heartbeat_at",
        "producer_lease_expires_at",
        "oldest_received_at",
        "oldest_provider_trade_at",
        "last_received_at",
        "lineage_valid",
        "gap_present",
        "valid_until",
    ];
    if aggregate.as_object().is_none_or(|object| {
        object.len() != FIELDS.len() || FIELDS.iter().any(|name| !object.contains_key(*name))
    }) || ["producer_present", "lineage_valid", "gap_present"]
        .iter()
        .any(|name| aggregate.get(*name).and_then(Value::as_bool).is_none())
        || aggregate
            .get("valid_until")
            .is_none_or(|value| !value.is_string())
    {
        return error_group(ReadFailure::Invalid, "stream");
    }
    let counts = ["demand_count", "live_count", "halted_count"]
        .map(|name| aggregate.get(name).and_then(Value::as_u64));
    let [Some(demand), Some(live), Some(halted)] = counts else {
        return error_group(ReadFailure::Invalid, "stream");
    };
    if halted > live || live > demand || demand > 30 {
        return error_group(ReadFailure::Invalid, "stream");
    }
    aggregate["live_count"] = json!(live - halted);
    let mut invalid_timestamp = false;
    for field in [
        "observed_at",
        "producer_status_at",
        "producer_heartbeat_at",
        "producer_lease_expires_at",
        "oldest_received_at",
        "oldest_provider_trade_at",
        "last_received_at",
        "valid_until",
    ] {
        if aggregate
            .get(field)
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            invalid_timestamp = true;
        }
        if let Some(value) = aggregate.get(field).and_then(Value::as_str) {
            if let Ok(value) = DateTime::parse_from_rfc3339(value) {
                aggregate[field] = json!(timestamp(value.with_timezone(&Utc)));
            } else {
                aggregate[field] = Value::Null;
                invalid_timestamp = true;
            }
        }
    }
    let safe_code = aggregate
        .get("status_code")
        .and_then(Value::as_str)
        .filter(|code| {
            matches!(
                *code,
                "FEATURE_DISABLED"
                    | "NO_ACTIVE_DEMAND"
                    | "CALENDAR_UNAVAILABLE"
                    | "SESSION_WINDOW_UNAVAILABLE"
                    | "SESSION_CLOSED"
                    | "AWAITING_FIRST_TRADE"
                    | "QUOTE_STALE"
                    | "CONNECTION_LOST"
                    | "RECONNECT_GAP"
                    | "SUBSCRIPTION_PENDING"
                    | "SUBSCRIPTION_REJECTED"
                    | "SUBSCRIPTION_AMBIGUOUS"
                    | "APPROVAL_UNAVAILABLE"
                    | "BUDGET_EXHAUSTED"
                    | "PRODUCER_UNAVAILABLE"
                    | "PIPELINE_LAG"
                    | "WIRE_SCHEMA_MISMATCH"
                    | "PROVIDER_RESPONSE_INVALID"
                    | "QUOTE_VALUE_INVALID"
                    | "MARKET_CLASS_UNSUPPORTED"
                    | "LOCAL_INGRESS_LIMIT"
                    | "RESYNC_REQUIRED"
                    | "ACCESS_REVOKED"
            )
        })
        .map(str::to_owned);
    let invalid_code = aggregate
        .get("status_code")
        .is_some_and(|code| !code.is_null() && safe_code.is_none());
    let invalid_connection =
        aggregate
            .get("connection_state")
            .is_none_or(|value| match value.as_str() {
                Some("CONNECTED" | "CONNECTING" | "DISCONNECTED" | "BACKOFF" | "STOPPED") => false,
                None if value.is_null()
                    && aggregate.get("producer_present").and_then(Value::as_bool)
                        == Some(false) =>
                {
                    false
                }
                _ => true,
            });
    let mut output = aggregate;
    if let Some(object) = output.as_object_mut() {
        object.insert(
            "result".to_owned(),
            json!(if invalid_code || invalid_timestamp || invalid_connection {
                "invalid"
            } else {
                "ok"
            }),
        );
        object.insert("owner_binding_valid".to_owned(), json!(true));
        if object.get("observed_at").is_none_or(Value::is_null) {
            return error_group(ReadFailure::Invalid, "stream");
        }
        object.insert(
            "status_code".to_owned(),
            safe_code.map(|code| json!(code)).unwrap_or(Value::Null),
        );
    }
    output
}

type StreamPinsRow = (
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn stream_pins(state: &ApiState, now: DateTime<Utc>) -> StreamPinsRow {
    let MarketStreamReadConfig::OwnerOnly(pins) = &state.cfg.owner_market_stream else {
        return (None, None, None, None, None, None);
    };
    let Some(window) = pins.window.as_ref() else {
        return (None, None, None, None, None, None);
    };
    let date = now
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).expect("KST"))
        .date_naive();
    let Some(entry) = window.entry(date) else {
        return (None, None, None, None, None, None);
    };
    if entry.evidence_retrieved_at > now
        || entry
            .evidence_retrieved_at
            .with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).expect("KST"))
            .date_naive()
            != date
    {
        return (None, None, None, None, None, None);
    }
    // A current validated CLOSED entry intentionally has no bounds. Retain
    // the real authorization/window pins so G1 can return safe demand evidence.
    let bounds = entry.utc_bounds();
    if bounds.is_none() && !entry.is_closed() {
        return (None, None, None, None, None, None);
    }
    (
        Some(pins.credential_slot_id),
        Some(pins.grant_id),
        Some(pins.contract_sha256.clone()),
        Some(window.window_contract_sha256().to_owned()),
        bounds.map(|(open, _)| open),
        bounds.map(|(_, close)| close),
    )
}

fn window_evidence(state: &ApiState, now: DateTime<Utc>) -> Value {
    let window = match &state.cfg.owner_market_stream {
        MarketStreamReadConfig::OwnerOnly(pins) => pins.window.as_deref(),
        MarketStreamReadConfig::Disabled => None,
    }
    .or_else(|| match &state.cfg.owner_intraday_quotes {
        OwnerIntradayQuoteReadConfig::OwnerOnly { window } => window.as_deref(),
        OwnerIntradayQuoteReadConfig::Disabled => None,
    });
    let Some(window) = window else {
        return json!({"result":"missing","observed_at":null,"source":"operational_v1"});
    };
    let date = now
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 60 * 60).expect("KST"))
        .date_naive();
    let Some(entry) = window.entry(date) else {
        return json!({"result":"missing","observed_at":timestamp(now),"source":"operational_v1",
            "activation_hash_matches":true,"metadata_safe":true});
    };
    let bounds = entry.utc_bounds();
    json!({
        "result":"ok","observed_at":timestamp(now),"source":"operational_v1",
        "activation_hash_matches":true,"metadata_safe":true,
        "entry":{
            "date":date.to_string(),
            "disposition":if entry.is_closed(){"closed"}else{"trading"},
            "open_at":bounds.map(|value|timestamp(value.0)),
            "close_at":bounds.map(|value|timestamp(value.1)),
            "evidence_retrieved_at":timestamp(entry.evidence_retrieved_at)
        }
    })
}

fn error_group(error: ReadFailure, kind: &str) -> Value {
    let mut result = group_failure(error.result());
    if let Some(object) = result.as_object_mut() {
        match kind {
            "queue" | "calendar" => {
                object.insert("rows".into(), json!([]));
            }
            "stream" => {
                object.insert("owner_binding_valid".into(), json!(false));
            }
            "research" => {
                object.insert("row".into(), Value::Null);
            }
            _ => {}
        }
    }
    result
}

/// Refresh the source-evidence input. This never runs a process command and
/// never opens provider configuration; only the fixed safe host file, existing
/// read-only window contract, and bounded serving-role database queries enter.
pub async fn refresh(
    state: &ApiState,
    actor: &Actor,
    session_hash: &str,
    now: DateTime<Utc>,
    shared_evidence: Option<&Value>,
) -> Value {
    let runtime = state.service_status_runtime.clone();
    let expected_release = state.cfg.code_commit.clone();
    let host = tokio::task::spawn_blocking(move || runtime.observe_host(now, &expected_release))
        .await
        .expect("host reader task completed");
    refresh_with_host(state, actor, session_hash, now, shared_evidence, &host).await
}

pub(crate) async fn refresh_with_host(
    state: &ApiState,
    actor: &Actor,
    session_hash: &str,
    now: DateTime<Utc>,
    shared_evidence: Option<&Value>,
    host: &crate::service_status::HostEvidence,
) -> Value {
    let start = Instant::now();
    let owner = actor_uuid(actor).ok();
    let host_read = host.read.clone();
    let host_error = host.error.map(|error| error.reason());
    let mut db = serde_json::Map::new();
    let queue = if let Some(owner) = owner {
        read_queue(
            &state.app_pool,
            actor,
            owner,
            start,
            host_read.pointer("/snapshot/policy/job_lease_seconds"),
        )
        .await
    } else {
        Err(ReadFailure::Denied)
    };
    db.insert(
        "queue".into(),
        queue.unwrap_or_else(|error| error_group(error, "queue")),
    );

    if let Some(shared) = shared_evidence {
        let shared_db = shared.get("db_read_results").and_then(Value::as_object);
        for name in ["etf", "rights", "candidate", "research"] {
            db.insert(
                name.to_owned(),
                shared_db
                    .and_then(|groups| groups.get(name))
                    .cloned()
                    .unwrap_or_else(|| error_group(ReadFailure::Unavailable, name)),
            );
        }
    } else {
        let etf = read_etf(&state.admin_pool, start).await;
        let mut etf_group = etf.unwrap_or_else(|error| error_group(error, "etf"));
        let rights = if etf_group.get("result").and_then(Value::as_str) == Some("ok") {
            read_rights(&state.admin_pool, &etf_group, start).await
        } else {
            Err(ReadFailure::Unavailable)
        };
        let rights_group = rights.unwrap_or_else(|error| error_group(error, "rights"));
        if let Some(object) = etf_group.as_object_mut() {
            object.insert(
                "rights_valid".into(),
                rights_group
                    .get("rights_valid")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            object.insert(
                "rights_result".into(),
                rights_group
                    .get("result")
                    .cloned()
                    .unwrap_or(json!("unavailable")),
            );
            object.insert(
                "rights_observed_at".into(),
                rights_group
                    .get("observed_at")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
        db.insert("etf".into(), etf_group);
        db.insert("rights".into(), rights_group);
        let candidate = read_candidate(&state.admin_pool, start).await;
        db.insert(
            "candidate".into(),
            candidate.unwrap_or_else(|error| error_group(error, "candidate")),
        );
        let research = read_research(&state.admin_pool, start).await;
        db.insert(
            "research".into(),
            research.unwrap_or_else(|error| error_group(error, "research")),
        );
    }

    let v2 = if let Some(owner) = owner {
        read_v2(&state.app_pool, actor, owner, start).await
    } else {
        Err(ReadFailure::Denied)
    };
    db.insert(
        "v2".into(),
        v2.unwrap_or_else(|error| error_group(error, "v2")),
    );
    let calendar = if owner.is_some() {
        read_calendar(&state.app_pool, actor, start).await
    } else {
        Err(ReadFailure::Denied)
    };
    db.insert(
        "calendar".into(),
        calendar.unwrap_or_else(|error| error_group(error, "calendar")),
    );
    let stream = if let Some(owner) = owner {
        read_stream(
            &state.app_pool,
            actor,
            owner,
            session_hash,
            state,
            now,
            start,
        )
        .await
    } else {
        Err(ReadFailure::Denied)
    };
    db.insert(
        "stream".into(),
        stream.unwrap_or_else(|error| error_group(error, "stream")),
    );
    json!({
        "fixture_version":1,
        "clock":timestamp(now),
        "expected_release":state.cfg.code_commit,
        "host_read":host_read,
        "host_error":host_error,
        "db_read_results":Value::Object(db),
        "window_read":window_evidence(state,now),
    })
}

pub fn extract_shared_evidence(input: &Value) -> Value {
    let source_groups = input.get("db_read_results").and_then(Value::as_object);
    let mut groups = serde_json::Map::new();
    for name in ["etf", "rights", "candidate", "research"] {
        if let Some(value) = source_groups.and_then(|groups| groups.get(name)) {
            groups.insert(name.to_owned(), value.clone());
        }
    }
    json!({
        "db_read_results": Value::Object(groups)
    })
}

#[allow(dead_code)]
fn _host_read_error_wire(error: HostReadError) -> &'static str {
    error.reason()
}

#[cfg(test)]
mod repair_tests {
    use super::*;

    fn aggregate(demand: Value, live: Value, halted: Value) -> Value {
        json!({"observed_at":"2032-02-12T01:00:00Z","demand_count":demand,"live_count":live,"halted_count":halted,
            "producer_present":true,"connection_state":"CONNECTED","status_code":null,"producer_status_at":null,
            "producer_heartbeat_at":"2032-02-12T00:59:59Z","producer_lease_expires_at":"2032-02-12T01:00:19Z",
            "oldest_received_at":"2032-02-12T00:59:58Z","oldest_provider_trade_at":"2032-02-12T00:59:57Z","last_received_at":"2032-02-12T00:59:59Z",
            "lineage_valid":true,"gap_present":false,"valid_until":"2032-02-12T01:00:09Z"})
    }

    #[test]
    fn sql_inclusive_counts_are_validated_before_one_exclusive_conversion() {
        let now = DateTime::parse_from_rfc3339("2032-02-12T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (d, l, h, exclusive) in [
            (2, 2, 0, 2),
            (2, 2, 2, 0),
            (2, 2, 1, 1),
            (2, 1, 1, 0),
            (0, 0, 0, 0),
        ] {
            let raw = aggregate(json!(d), json!(l), json!(h));
            let output = adapt_stream_aggregate(Some(raw.clone()), now);
            assert_eq!(output["result"], "ok");
            assert_eq!(output["demand_count"], d);
            assert_eq!(output["live_count"], exclusive);
            assert_eq!(output["halted_count"], h);
            for name in [
                "observed_at",
                "producer_heartbeat_at",
                "producer_lease_expires_at",
                "oldest_received_at",
                "oldest_provider_trade_at",
                "valid_until",
            ] {
                assert_eq!(
                    output[name], raw[name],
                    "{name} must retain its actual instant"
                );
            }
        }
        for (d, l, h) in [
            (json!(2), json!(1), json!(2)),
            (json!(2), json!(3), json!(0)),
            (json!(31), json!(0), json!(0)),
            (json!(-1), json!(0), json!(0)),
            (json!(2), json!(1.5), json!(0)),
            (json!(2), Value::Null, json!(0)),
        ] {
            assert_eq!(
                adapt_stream_aggregate(Some(aggregate(d, l, h)), now)["result"],
                "invalid"
            );
        }
        let missing = adapt_stream_aggregate(None, now);
        assert_eq!(missing["result"], "ok");
        assert_eq!(missing["owner_binding_valid"], false);
        assert!(missing["demand_count"].is_null());
    }
}
