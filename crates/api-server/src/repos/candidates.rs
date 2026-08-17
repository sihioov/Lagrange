//! Read-only common candidate output and actor-owned saved screener queries.

use crate::actor_tx::{actor_uuid, begin_actor_tx};
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct CandidateRunRow {
    pub id: Uuid,
    pub universe_key: String,
    pub as_of_date: NaiveDate,
    pub cutoff_at: DateTime<Utc>,
    pub computation_seq: i32,
    pub scoring_config_version: String,
    pub scoring_config_sha256: String,
    pub input_identity_sha256: String,
    pub universe_snapshot_id: Uuid,
    pub price_dataset_version_id: Uuid,
    pub price_curated_version: i32,
    pub price_manifest_sha256: String,
    pub status_dataset_version_id: Uuid,
    pub status_manifest_sha256: String,
    pub flow_dataset_version_id: Uuid,
    pub flow_manifest_sha256: String,
    pub fundamental_dataset_version_id: Uuid,
    pub fundamental_manifest_sha256: String,
    pub sector_version_id: Uuid,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CandidateFeedRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub universe_key: String,
    pub as_of_date: NaiveDate,
    pub computation_seq: i32,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CandidateAnalysisRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub universe_key: String,
    pub instrument_id: String,
    pub instrument_name: Option<String>,
    pub sector_code: String,
    pub fundamental_profile: String,
    pub eligible: bool,
    pub exclusion_codes: Value,
    pub flow_score: Option<f64>,
    pub fundamental_score: Option<f64>,
    pub technical_score: Option<f64>,
    pub total_score: Option<f64>,
    /// Exact PostgreSQL numeric text for keyset cursor anchors. Rebuilding
    /// this value from `f64` can move a page boundary.
    pub total_score_text: Option<String>,
    pub flow_coverage: f64,
    pub fundamental_coverage: f64,
    pub technical_coverage: f64,
    pub evidence_strength: String,
    pub rank: Option<i32>,
    pub normalization_scope: String,
    pub factors_json: Value,
    pub scenarios_json: Value,
    pub provenance_json: Value,
    pub content_sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SavedScreenRow {
    pub id: Uuid,
    pub name: String,
    pub criteria_schema_version: i32,
    pub criteria_json: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ScreenFilter {
    /// The immutable run-set selected for this request. The order is the
    /// registry order and is part of the cursor capability.
    pub run_set: Vec<(String, Uuid)>,
    pub as_of_date: Option<NaiveDate>,
    pub sectors: Vec<String>,
    pub evidence: Vec<String>,
    pub min_total_score: Option<f64>,
    pub min_flow_score: Option<f64>,
    pub min_fundamental_score: Option<f64>,
    pub min_technical_score: Option<f64>,
    /// Exact `numeric` text from the cursor anchor.
    pub after_universe: Option<String>,
    pub after_score: Option<String>,
    pub after_instrument: Option<String>,
    pub limit: usize,
}

/// Exact source-rights identity copied onto a published candidate run. The
/// API gate and the response both use this row rather than guessing a license
/// from the logical dataset name.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct CandidateLicenseAttribution {
    pub source: String,
    pub dataset_id: String,
    pub license_ref: String,
    pub entitlement_id: Uuid,
    pub contract_reference: String,
    pub contract_document_sha256: String,
}

#[derive(Debug, Clone)]
pub struct CandidateRepo {
    pool: sqlx::PgPool,
    seoul_today: fn() -> NaiveDate,
    candidate_eod_ready: fn() -> bool,
}

impl CandidateRepo {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self::with_close_clock(
            pool,
            crate::http::state::system_seoul_today,
            crate::http::state::system_candidate_eod_ready,
        )
    }

    pub fn with_close_clock(
        pool: sqlx::PgPool,
        seoul_today: fn() -> NaiveDate,
        candidate_eod_ready: fn() -> bool,
    ) -> Self {
        Self {
            pool,
            seoul_today,
            candidate_eod_ready,
        }
    }

    /// Return the newest KRX trading session for which both published
    /// calendar provenance and a credentialed EOD batch are present.
    ///
    /// A calendar row alone is not proof that the close happened: a current
    /// trading calendar can exist before the session closes. Requiring the
    /// same publication prerequisites used by the worker keeps API freshness
    /// independent of the process clock and naturally handles weekends,
    /// holidays, and pre-close dates.
    pub async fn latest_confirmed_krx_close(&self) -> TenancyResult<Option<NaiveDate>> {
        sqlx::query_scalar::<_, NaiveDate>(
            "WITH expected AS MATERIALIZED (
                 SELECT max(calendar.session_date) AS session_date
                   FROM trading_calendars AS calendar
                  WHERE calendar.exchange = 'KRX'
                    AND calendar.session_type = 'TRADING'
                    AND calendar.timezone = 'Asia/Seoul'
                    AND calendar.session_date <= $1
                    AND (calendar.session_date < $1 OR $2)
                    AND calendar.source_batch_id IS NOT NULL
                    AND calendar.content_sha256 IS NOT NULL
                    AND calendar.retrieved_at IS NOT NULL)
             SELECT expected.session_date
               FROM expected
              WHERE expected.session_date IS NOT NULL
                AND EXISTS (
                    SELECT 1 FROM data_batches AS batch
                     WHERE batch.batch_date = expected.session_date
                       AND batch.provider = 'KRX'
                       AND batch.market = 'KR'
                       AND batch.kind = 'EOD'
                       AND batch.fetch_mode = 'credentialed'
                       AND batch.source_batch_id IS NOT NULL
                       AND batch.source_file_name IS NOT NULL)",
        )
        .bind((self.seoul_today)())
        .bind((self.candidate_eod_ready)())
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)
    }

    /// Classify a published run against the latest DB-confirmed KRX close.
    /// A successful research response can only be READY or STALE; blocked
    /// source state is represented by the entitlement error path instead.
    pub async fn freshness_state(&self, as_of: NaiveDate) -> TenancyResult<&'static str> {
        let latest = self.latest_confirmed_krx_close().await?;
        Ok(if latest == Some(as_of) {
            "READY"
        } else {
            "STALE"
        })
    }

    pub async fn latest_feed(
        &self,
        universe_key: &str,
        as_of: Option<NaiveDate>,
    ) -> TenancyResult<Option<(CandidateFeedRow, CandidateRunRow)>> {
        let feed = sqlx::query_as::<_, CandidateFeedRow>(
            "SELECT feed.id, feed.run_id, feed.universe_key,
                    feed.as_of_date, feed.computation_seq, feed.published_at
             FROM candidate_feed_snapshots AS feed
             WHERE feed.status = 'PUBLISHED'
               AND feed.universe_key = $1
               AND ($2::date IS NULL OR feed.as_of_date = $2)
             ORDER BY feed.as_of_date DESC, feed.computation_seq DESC
             LIMIT 1",
        )
        .bind(universe_key)
        .bind(as_of)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let Some(feed) = feed else { return Ok(None) };
        let run = self.run_by_id(feed.run_id).await?;
        Ok(Some((feed, run)))
    }

    pub async fn feed_items(&self, feed_id: Uuid) -> TenancyResult<Vec<CandidateAnalysisRow>> {
        sqlx::query_as::<_, CandidateAnalysisRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {ANALYSIS_COLUMNS}
               FROM candidate_feed_items AS item
               JOIN stock_analysis_snapshots AS snapshot
                 ON snapshot.id = item.stock_analysis_snapshot_id
               JOIN stock_analysis_runs AS run ON run.id = snapshot.run_id
               JOIN instruments AS instrument ON instrument.id = snapshot.instrument_id
              WHERE item.feed_id = $1
              ORDER BY item.rank"
        )))
        .bind(feed_id)
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)
    }

    pub async fn instrument_analysis(
        &self,
        instrument_id: &str,
        universe_key: &str,
        as_of: Option<NaiveDate>,
    ) -> TenancyResult<Option<(CandidateRunRow, CandidateAnalysisRow)>> {
        let row = sqlx::query_as::<_, CandidateAnalysisRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {ANALYSIS_COLUMNS}
             FROM stock_analysis_snapshots AS snapshot
             JOIN stock_analysis_runs AS run ON run.id = snapshot.run_id
             JOIN candidate_feed_snapshots AS feed ON feed.run_id = run.id
             JOIN instruments AS instrument ON instrument.id = snapshot.instrument_id
               WHERE snapshot.instrument_id = $1 AND feed.status = 'PUBLISHED'
               AND run.universe_key = $2
               AND ($3::date IS NULL OR run.as_of_date = $3)
             ORDER BY run.as_of_date DESC, run.computation_seq DESC LIMIT 1"
        )))
        .bind(instrument_id)
        .bind(universe_key)
        .bind(as_of)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        let Some(analysis) = row else { return Ok(None) };
        let run = self.run_by_id(analysis.run_id).await?;
        Ok(Some((run, analysis)))
    }

    /// Resolve the newest published run for each requested universe. This is
    /// called only when a screener request starts without a cursor; subsequent
    /// pages carry the exact run-set in their signed capability.
    pub async fn latest_runs(
        &self,
        universes: &[String],
        as_of: Option<NaiveDate>,
    ) -> TenancyResult<Vec<CandidateRunRow>> {
        let rows: Vec<CandidateRunRow> = sqlx::query_as(
            "SELECT DISTINCT ON (registry.universe_key)
                    run.id, run.universe_key, run.as_of_date, run.cutoff_at,
                    run.computation_seq, run.scoring_config_version,
                    run.scoring_config_sha256, run.input_identity_sha256,
                    run.universe_snapshot_id, run.price_dataset_version_id,
                    run.price_curated_version, run.price_manifest_sha256,
                    run.status_dataset_version_id, run.status_manifest_sha256,
                    run.flow_dataset_version_id, run.flow_manifest_sha256,
                    run.fundamental_dataset_version_id,
                    run.fundamental_manifest_sha256, run.sector_version_id,
                    run.published_at
               FROM stock_analysis_runs AS run
               JOIN candidate_feed_snapshots AS feed
                 ON feed.run_id = run.id
                AND feed.universe_key = run.universe_key
                AND feed.status = 'PUBLISHED'
               JOIN candidate_universe_registry AS registry
                 ON registry.universe_key = run.universe_key
                AND registry.enabled
              WHERE run.status = 'SUCCEEDED'
                AND run.universe_key = ANY($1::text[])
                AND ($2::date IS NULL OR run.as_of_date = $2)
              ORDER BY registry.universe_key, registry.sort_order,
                       run.as_of_date DESC, run.computation_seq DESC",
        )
        .bind(universes)
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    /// Read a run's immutable universe identity for the legacy `run_id`
    /// request path. The caller validates that it matches the requested
    /// universe instead of silently falling back to another feed.
    pub async fn run_universe(&self, run_id: Uuid) -> TenancyResult<Option<String>> {
        sqlx::query_scalar::<_, String>(
            "SELECT universe_key
               FROM stock_analysis_runs
              WHERE id = $1 AND status = 'SUCCEEDED'",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)
    }

    pub async fn screen(
        &self,
        filter: &ScreenFilter,
    ) -> TenancyResult<Vec<(CandidateRunRow, Vec<CandidateAnalysisRow>)>> {
        if filter.run_set.is_empty() {
            return Err(TenancyError::NotFound);
        }

        // A cursor pins every run in the request. Do not re-resolve the
        // latest feed here: a correction may supersede the active feed while
        // a client is paging through the original immutable run-set.
        let run_ids = filter.run_set.iter().map(|(_, id)| *id).collect::<Vec<_>>();
        let runs: Vec<CandidateRunRow> = sqlx::query_as(
            "SELECT id, universe_key, as_of_date, cutoff_at, computation_seq,
                    scoring_config_version, scoring_config_sha256, input_identity_sha256,
                    universe_snapshot_id, price_dataset_version_id, price_curated_version,
                    price_manifest_sha256, status_dataset_version_id, status_manifest_sha256,
                    flow_dataset_version_id, flow_manifest_sha256,
                    fundamental_dataset_version_id, fundamental_manifest_sha256,
                    sector_version_id, published_at
               FROM stock_analysis_runs
              WHERE status = 'SUCCEEDED'
                AND id = ANY($1::uuid[])
                AND ($2::date IS NULL OR as_of_date = $2)",
        )
        .bind(&run_ids)
        .bind(filter.as_of_date)
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;

        let mut ordered_runs = Vec::with_capacity(filter.run_set.len());
        for (expected_universe, expected_run_id) in &filter.run_set {
            let run = runs
                .iter()
                .find(|run| run.id == *expected_run_id && run.universe_key == *expected_universe)
                .cloned()
                .ok_or(TenancyError::NotFound)?;
            ordered_runs.push(run);
        }

        let probe = i64::try_from(filter.limit.saturating_add(1)).unwrap_or(i64::MAX);
        let mut blocks = Vec::with_capacity(ordered_runs.len());
        for run in ordered_runs {
            // Rows in blocks after the cursor's universe are unanchored. The
            // anchor only applies inside the block where the cursor stopped.
            let anchored = filter
                .after_universe
                .as_deref()
                .map(|universe| universe == run.universe_key)
                .unwrap_or(false);
            let before_anchor = filter.after_universe.as_deref().is_some_and(|universe| {
                run.universe_key != universe
                    && crate::contract::UniverseKey::parse(&run.universe_key)
                        .ok()
                        .is_some_and(|key| {
                            crate::contract::UniverseKey::parse(universe)
                                .ok()
                                .is_some_and(|anchor| key.sort_order() < anchor.sort_order())
                        })
            });
            if before_anchor {
                blocks.push((run, Vec::new()));
                continue;
            }
            let after_score = if anchored {
                filter.after_score.clone()
            } else {
                None
            };
            let after_instrument = if anchored {
                filter.after_instrument.clone()
            } else {
                None
            };
            let rows = sqlx::query_as::<_, CandidateAnalysisRow>(sqlx::AssertSqlSafe(format!(
                "SELECT {ANALYSIS_COLUMNS}
                   FROM stock_analysis_snapshots AS snapshot
                   JOIN stock_analysis_runs AS run ON run.id = snapshot.run_id
                   JOIN instruments AS instrument ON instrument.id = snapshot.instrument_id
                  WHERE snapshot.run_id = $1 AND snapshot.eligible
                    AND (cardinality($2::text[]) = 0 OR snapshot.sector_code = ANY($2))
                    AND (cardinality($3::text[]) = 0 OR snapshot.evidence_strength = ANY($3))
                    AND ($4::double precision IS NULL OR snapshot.total_score >= $4)
                    AND ($5::double precision IS NULL OR snapshot.flow_score >= $5)
                    AND ($6::double precision IS NULL OR snapshot.fundamental_score >= $6)
                    AND ($7::double precision IS NULL OR snapshot.technical_score >= $7)
                    AND (
                        $8::numeric IS NULL
                        OR snapshot.total_score < $8::numeric
                        OR (snapshot.total_score = $8::numeric AND snapshot.instrument_id > $9)
                    )
                  ORDER BY snapshot.total_score DESC, snapshot.instrument_id
                  LIMIT $10"
            )))
            .bind(run.id)
            .bind(&filter.sectors)
            .bind(&filter.evidence)
            .bind(filter.min_total_score)
            .bind(filter.min_flow_score)
            .bind(filter.min_fundamental_score)
            .bind(filter.min_technical_score)
            .bind(after_score)
            .bind(&after_instrument)
            .bind(probe)
            .fetch_all(&self.pool)
            .await
            .map_err(TenancyError::from_sqlx)?;
            blocks.push((run, rows));
        }
        if blocks.is_empty() {
            return Err(TenancyError::NotFound);
        }
        Ok(blocks)
    }

    pub async fn license_attributions(
        &self,
        run_id: Uuid,
    ) -> TenancyResult<Vec<CandidateLicenseAttribution>> {
        let rows: Vec<CandidateLicenseAttribution> = sqlx::query_as(
            "SELECT source,dataset_id,license_ref,entitlement_id,
                    contract_reference,contract_document_sha256
               FROM public.candidate_published_source_attributions($1)",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    pub async fn dataset_ids(&self, run_id: Uuid) -> TenancyResult<Vec<String>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT dataset_id
               FROM public.candidate_published_source_attributions($1)
              ORDER BY dataset_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        if rows.is_empty() {
            Err(TenancyError::NotFound)
        } else {
            Ok(rows)
        }
    }

    async fn run_by_id(&self, run_id: Uuid) -> TenancyResult<CandidateRunRow> {
        let row = sqlx::query_as(
            "SELECT id, universe_key, as_of_date, cutoff_at, computation_seq,
                    scoring_config_version, scoring_config_sha256, input_identity_sha256,
                    universe_snapshot_id, price_dataset_version_id, price_curated_version,
                    price_manifest_sha256, status_dataset_version_id, status_manifest_sha256,
                    flow_dataset_version_id, flow_manifest_sha256,
                    fundamental_dataset_version_id, fundamental_manifest_sha256,
                    sector_version_id, published_at
               FROM stock_analysis_runs WHERE id = $1 AND status = 'SUCCEEDED'",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    pub async fn list_screens(&self, actor: &Actor) -> TenancyResult<Vec<SavedScreenRow>> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let rows = sqlx::query_as(
            "SELECT id, name, criteria_schema_version, criteria_json, created_at, updated_at
               FROM screener_saved_screens ORDER BY updated_at DESC, id",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(rows)
    }

    pub async fn get_screen(&self, actor: &Actor, id: Uuid) -> TenancyResult<SavedScreenRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as(
            "SELECT id, name, criteria_schema_version, criteria_json, created_at, updated_at
               FROM screener_saved_screens WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    pub async fn create_screen(
        &self,
        actor: &Actor,
        name: &str,
        criteria: &Value,
    ) -> TenancyResult<SavedScreenRow> {
        let owner = actor_uuid(actor)?;
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as(
            "INSERT INTO screener_saved_screens
                (owner_user_id, name, criteria_schema_version, criteria_json)
             VALUES ($1, $2, 2, $3)
             RETURNING id, name, criteria_schema_version, criteria_json, created_at, updated_at",
        )
        .bind(owner)
        .bind(name)
        .bind(criteria)
        .fetch_one(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        Ok(row)
    }

    pub async fn update_screen(
        &self,
        actor: &Actor,
        id: Uuid,
        name: &str,
        criteria: &Value,
    ) -> TenancyResult<SavedScreenRow> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let row = sqlx::query_as(
            "UPDATE screener_saved_screens
                SET name = $2, criteria_schema_version = 2,
                    criteria_json = $3, updated_at = clock_timestamp()
              WHERE id = $1
              RETURNING id, name, criteria_schema_version, criteria_json, created_at, updated_at",
        )
        .bind(id)
        .bind(name)
        .bind(criteria)
        .fetch_optional(&mut *tx)
        .await
        .map_err(TenancyError::from_sqlx)?;
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        crate::error::map_optional(row)
    }

    pub async fn delete_screen(&self, actor: &Actor, id: Uuid) -> TenancyResult<()> {
        let mut tx = begin_actor_tx(&self.pool, actor).await?;
        let changed = sqlx::query("DELETE FROM screener_saved_screens WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(TenancyError::from_sqlx)?
            .rows_affected();
        tx.commit().await.map_err(TenancyError::from_sqlx)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(TenancyError::NotFound)
        }
    }
}

const ANALYSIS_COLUMNS: &str = "
    snapshot.id, snapshot.run_id, run.universe_key, snapshot.instrument_id,
    instrument.name AS instrument_name, snapshot.sector_code,
    snapshot.fundamental_profile, snapshot.eligible, snapshot.exclusion_codes,
    snapshot.flow_score::double precision AS flow_score,
    snapshot.fundamental_score::double precision AS fundamental_score,
    snapshot.technical_score::double precision AS technical_score,
    snapshot.total_score::double precision AS total_score,
    snapshot.total_score::text AS total_score_text,
    snapshot.flow_coverage::double precision AS flow_coverage,
    snapshot.fundamental_coverage::double precision AS fundamental_coverage,
    snapshot.technical_coverage::double precision AS technical_coverage,
    snapshot.evidence_strength, snapshot.rank, snapshot.normalization_scope,
    snapshot.factors_json, snapshot.scenarios_json, snapshot.provenance_json,
    snapshot.content_sha256";

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn production_constructor_uses_the_shared_seoul_close_clock() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://app:unused@127.0.0.1/unused")
            .expect("lazy PostgreSQL URL");
        let repo = CandidateRepo::new(pool);
        assert!(std::ptr::fn_addr_eq(
            repo.seoul_today,
            crate::http::state::system_seoul_today as fn() -> NaiveDate,
        ));
        assert!(std::ptr::fn_addr_eq(
            repo.candidate_eod_ready,
            crate::http::state::system_candidate_eod_ready as fn() -> bool,
        ));
    }
}
