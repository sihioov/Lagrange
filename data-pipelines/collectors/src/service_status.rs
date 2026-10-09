//! Fenced, best-effort EOD execution status writes.
//!
//! This module is used only by the parent scheduled research worker and the
//! separate credentialed bounded backfill. Raw-only and internal child paths
//! do not construct the repository.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use market_data::providers::kis::KR_ETF_CORE_SYMBOLS;
use sqlx::PgPool;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
pub const RESEARCH_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchFailureCode {
    PipelineFailed,
    PublicationFailed,
    RightsUnavailable,
    ExecutionInterrupted,
}

impl ResearchFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PipelineFailed => "PIPELINE_FAILED",
            Self::PublicationFailed => "PUBLICATION_FAILED",
            Self::RightsUnavailable => "RIGHTS_UNAVAILABLE",
            Self::ExecutionInterrupted => "EXECUTION_INTERRUPTED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchAttemptCompletion {
    Published { target_session: NaiveDate },
    NoWork,
    Failed(ResearchFailureCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResearchAttemptFence {
    pub attempt_started_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResearchStatusWriteError {
    #[error("research status database write failed")]
    Database,
    #[error("research status database write timed out")]
    Timeout,
}

#[derive(Debug, Clone)]
pub struct ResearchExecutionStatusRepository {
    pool: PgPool,
}

impl ResearchExecutionStatusRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record an actual attempt only when the preceding attempt is terminal or
    /// its persisted heartbeat is strictly older than 90 seconds. The captured
    /// database timestamp is the new fence; observers never acquire one.
    pub async fn start(
        &self,
        target_session: NaiveDate,
    ) -> Result<Option<ResearchAttemptFence>, ResearchStatusWriteError> {
        let started_at = tokio::time::timeout(
            WRITE_TIMEOUT,
            sqlx::query_scalar::<_, DateTime<Utc>>(
                "WITH db_clock AS MATERIALIZED (
                     SELECT pg_catalog.clock_timestamp() AS now
                 )
                 INSERT INTO public.research_execution_status (
                     id, attempt_started_at, heartbeat_at, attempt_finished_at,
                     attempt_state, target_session, updated_at
                 )
                 SELECT 'kis-eod', db_clock.now, db_clock.now, NULL,
                        'RUNNING', $1, db_clock.now
                   FROM db_clock
                 ON CONFLICT (id) DO UPDATE
                     SET attempt_started_at = EXCLUDED.attempt_started_at,
                         heartbeat_at = EXCLUDED.heartbeat_at,
                         attempt_finished_at = NULL,
                         attempt_state = 'RUNNING',
                         failure_code = NULL,
                         target_session = EXCLUDED.target_session,
                         updated_at = EXCLUDED.updated_at
                   WHERE public.research_execution_status.attempt_state <> 'RUNNING'
                      OR public.research_execution_status.heartbeat_at
                            < EXCLUDED.heartbeat_at - INTERVAL '90 seconds'
                 RETURNING attempt_started_at",
            )
            .bind(target_session)
            .fetch_optional(&self.pool),
        )
        .await
        .map_err(|_| ResearchStatusWriteError::Timeout)?
        .map_err(|_| ResearchStatusWriteError::Database)?;

        Ok(started_at.map(|attempt_started_at| ResearchAttemptFence { attempt_started_at }))
    }

    /// One conditional heartbeat. A stale or lost fence never updates a newer
    /// attempt, and callers do not retry pipeline work when this returns false.
    pub async fn heartbeat(
        &self,
        fence: ResearchAttemptFence,
    ) -> Result<bool, ResearchStatusWriteError> {
        let updated = tokio::time::timeout(
            WRITE_TIMEOUT,
            sqlx::query(
                "WITH db_clock AS MATERIALIZED (
                     SELECT pg_catalog.clock_timestamp() AS now
                 )
                 UPDATE public.research_execution_status AS status
                    SET heartbeat_at = db_clock.now,
                        updated_at = db_clock.now
                   FROM db_clock
                  WHERE status.id = 'kis-eod'
                    AND status.attempt_state = 'RUNNING'
                    AND status.attempt_started_at = $1
                 RETURNING status.id",
            )
            .bind(fence.attempt_started_at)
            .fetch_optional(&self.pool),
        )
        .await
        .map_err(|_| ResearchStatusWriteError::Timeout)?
        .map_err(|_| ResearchStatusWriteError::Database)?;
        Ok(updated.is_some())
    }

    /// Finish using the original fence. Curated success is derived from an
    /// exact current publication binding, including all fixed ETF sessions;
    /// a Raw batch or a child process result cannot set last_success_at.
    pub async fn finish(
        &self,
        fence: ResearchAttemptFence,
        completion: ResearchAttemptCompletion,
    ) -> Result<bool, ResearchStatusWriteError> {
        match completion {
            ResearchAttemptCompletion::Published { target_session } => {
                self.finish_published(fence, target_session).await
            }
            ResearchAttemptCompletion::NoWork => self.finish_no_work(fence).await,
            ResearchAttemptCompletion::Failed(code) => self.finish_failed(fence, code).await,
        }
    }

    async fn finish_no_work(
        &self,
        fence: ResearchAttemptFence,
    ) -> Result<bool, ResearchStatusWriteError> {
        let updated = tokio::time::timeout(
            WRITE_TIMEOUT,
            sqlx::query(
                "WITH db_clock AS MATERIALIZED (
                     SELECT pg_catalog.clock_timestamp() AS now
                 )
                 UPDATE public.research_execution_status AS status
                    SET attempt_finished_at = db_clock.now,
                        attempt_state = 'NO_WORK',
                        failure_code = NULL,
                        updated_at = db_clock.now
                   FROM db_clock
                  WHERE status.id = 'kis-eod'
                    AND status.attempt_state = 'RUNNING'
                    AND status.attempt_started_at = $1
                 RETURNING status.id",
            )
            .bind(fence.attempt_started_at)
            .fetch_optional(&self.pool),
        )
        .await
        .map_err(|_| ResearchStatusWriteError::Timeout)?
        .map_err(|_| ResearchStatusWriteError::Database)?;
        Ok(updated.is_some())
    }

    async fn finish_failed(
        &self,
        fence: ResearchAttemptFence,
        code: ResearchFailureCode,
    ) -> Result<bool, ResearchStatusWriteError> {
        let updated = tokio::time::timeout(
            WRITE_TIMEOUT,
            sqlx::query(
                "WITH db_clock AS MATERIALIZED (
                     SELECT pg_catalog.clock_timestamp() AS now
                 )
                 UPDATE public.research_execution_status AS status
                    SET attempt_finished_at = db_clock.now,
                        attempt_state = 'FAILED',
                        last_failure_at = db_clock.now,
                        failure_code = $2,
                        updated_at = db_clock.now
                   FROM db_clock
                  WHERE status.id = 'kis-eod'
                    AND status.attempt_state = 'RUNNING'
                    AND status.attempt_started_at = $1
                 RETURNING status.id",
            )
            .bind(fence.attempt_started_at)
            .bind(code.as_str())
            .fetch_optional(&self.pool),
        )
        .await
        .map_err(|_| ResearchStatusWriteError::Timeout)?
        .map_err(|_| ResearchStatusWriteError::Database)?;
        Ok(updated.is_some())
    }

    async fn finish_published(
        &self,
        fence: ResearchAttemptFence,
        target_session: NaiveDate,
    ) -> Result<bool, ResearchStatusWriteError> {
        let expected_instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<Vec<_>>();
        let updated = tokio::time::timeout(
            WRITE_TIMEOUT,
            sqlx::query(
                "WITH db_clock AS MATERIALIZED (
                     SELECT pg_catalog.clock_timestamp() AS now
                 ),
                 exact_publication AS MATERIALIZED (
                     SELECT pg_catalog.max(batch.published_at) AS published_at
                       FROM public.candidate_price_publications AS price
                       JOIN public.dataset_versions AS dataset
                         ON dataset.id = price.dataset_version_id
                        AND dataset.version = price.dataset_version
                        AND dataset.manifest_sha256 = price.manifest_sha256
                       JOIN public.candidate_raw_batch_datasets AS binding
                         ON binding.dataset_version_id = price.dataset_version_id
                        AND binding.surface = 'price'
                        AND binding.response_kind = 'bars'
                       JOIN public.candidate_raw_batch_publications AS batch
                         ON batch.batch_id = binding.batch_id
                        AND batch.surface = binding.surface
                        AND batch.state = 'PUBLISHED'
                        AND batch.fetch_mode = 'credentialed'
                      WHERE dataset.dataset_id = 'krx_eod_bars'
                        AND dataset.status IN ('READY', 'WARNING')
                        AND price.first_session <= $2
                        AND price.last_session >= $2
                        AND price.available_at <= (SELECT now FROM db_clock)
                        AND price.retrieved_at <= (SELECT now FROM db_clock)
                        AND batch.published_at <= (SELECT now FROM db_clock)
                        AND public.price_dataset_entitlement_is_valid(
                            price.entitlement_id, price.license_ref,
                            price.first_session, price.last_session
                        )
                        AND NOT EXISTS (
                            SELECT 1
                              FROM pg_catalog.unnest($3::text[]) AS expected(instrument_id)
                             WHERE NOT EXISTS (
                                 SELECT 1
                                   FROM public.candidate_price_instrument_sessions AS covered
                                  WHERE covered.dataset_version_id = price.dataset_version_id
                                    AND covered.instrument_id = expected.instrument_id
                                    AND covered.session_date = $2
                             )
                        )
                 )
                 UPDATE public.research_execution_status AS status
                    SET attempt_finished_at = db_clock.now,
                        attempt_state = CASE
                            WHEN exact_publication.published_at IS NULL THEN 'FAILED'
                            ELSE 'SUCCEEDED'
                        END,
                        last_success_at = CASE
                            WHEN exact_publication.published_at IS NULL
                                THEN status.last_success_at
                            ELSE exact_publication.published_at
                        END,
                        last_failure_at = CASE
                            WHEN exact_publication.published_at IS NULL
                                THEN db_clock.now
                            ELSE status.last_failure_at
                        END,
                        failure_code = CASE
                            WHEN exact_publication.published_at IS NULL
                                THEN 'PUBLICATION_FAILED'
                            ELSE NULL
                        END,
                        published_session = CASE
                            WHEN exact_publication.published_at IS NULL
                                THEN status.published_session
                            ELSE $2
                        END,
                        updated_at = db_clock.now
                   FROM db_clock, exact_publication
                  WHERE status.id = 'kis-eod'
                    AND status.attempt_state = 'RUNNING'
                    AND status.attempt_started_at = $1
                 RETURNING status.id",
            )
            .bind(fence.attempt_started_at)
            .bind(target_session)
            .bind(expected_instruments)
            .fetch_optional(&self.pool),
        )
        .await
        .map_err(|_| ResearchStatusWriteError::Timeout)?
        .map_err(|_| ResearchStatusWriteError::Database)?;
        Ok(updated.is_some())
    }
}

/// Owns the confirmed fence and its serialized heartbeat task for one parent
/// attempt. Failure to create, pulse, or finish this status is intentionally
/// swallowed at the boundary; collection runs once and keeps its own result.
pub struct ResearchExecutionAttempt {
    repository: ResearchExecutionStatusRepository,
    fence: ResearchAttemptFence,
    stop_heartbeat: Option<oneshot::Sender<()>>,
    heartbeat_task: Option<JoinHandle<()>>,
}

impl ResearchExecutionAttempt {
    pub async fn start(
        repository: ResearchExecutionStatusRepository,
        target_session: NaiveDate,
    ) -> Option<Self> {
        let fence = repository.start(target_session).await.ok().flatten()?;
        let (stop_heartbeat, mut stop_rx) = oneshot::channel();
        let heartbeat_repository = repository.clone();
        let heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(RESEARCH_HEARTBEAT_INTERVAL);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let _ = heartbeat_repository.heartbeat(fence).await;
                    }
                    _ = &mut stop_rx => break,
                }
            }
        });
        Some(Self {
            repository,
            fence,
            stop_heartbeat: Some(stop_heartbeat),
            heartbeat_task: Some(heartbeat_task),
        })
    }

    pub async fn finish(mut self, completion: ResearchAttemptCompletion) {
        if let Some(stop) = self.stop_heartbeat.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.heartbeat_task.take() {
            let _ = task.await;
        }
        let _ = self.repository.finish(self.fence, completion).await;
    }
}
