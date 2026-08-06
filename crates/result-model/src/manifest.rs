//! T3 backtest-result DB manifest writer (migration 0006, plan Todo 20).
//!
//! The `nt/backtest-worker` normalizer emits `manifest.json` (one row per
//! backtest_runs, plus metrics/warnings/artifact rows); this module is the
//! Rust-facing publisher: it validates the manifest and writes it via sqlx as
//! a SHORT transaction (the simulation work already happened - T19
//! convention: never hold a transaction during work).
//!
//! Idempotency: `backtest_runs.id` is caller-supplied (the queue consumer
//! derives a deterministic run id from the job), so a retried job re-writes
//! the SAME run id and `ON CONFLICT (id) DO NOTHING` makes the publish a
//! no-op instead of duplicating rows. The publisher never fabricates a run
//! id; the worker gate decides whether publication may happen at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::types::Uuid;

use domain::ReportedStat;

use crate::Warning;

/// A typed manifest-writer failure.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid manifest: {0}")]
    Invalid(String),
    #[error("idempotent re-publish mismatch: {0}")]
    IdempotencyMismatch(String),
}

/// The nine Parquet artifact kinds (design §6.10; `result_artifacts` CHECK).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactType {
    EquityCurve,
    DrawdownCurve,
    MonthlyReturns,
    Orders,
    Fills,
    Positions,
    CashLedger,
    Fees,
    Benchmark,
}

impl ArtifactType {
    /// The exact `result_artifacts.artifact_type` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EquityCurve => "EQUITY_CURVE",
            Self::DrawdownCurve => "DRAWDOWN_CURVE",
            Self::MonthlyReturns => "MONTHLY_RETURNS",
            Self::Orders => "ORDERS",
            Self::Fills => "FILLS",
            Self::Positions => "POSITIONS",
            Self::CashLedger => "CASH_LEDGER",
            Self::Fees => "FEES",
            Self::Benchmark => "BENCHMARK",
        }
    }
}

/// One Parquet artifact row (migration 0006 `result_artifacts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub artifact_type: ArtifactType,
    pub parquet_path: String,
    pub row_count: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub summary_json: serde_json::Value,
}

/// One run row (migration 0006 `backtest_runs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub job_id: Option<Uuid>,
    pub strategy_id: String,
    pub strategy_version: String,
    pub dataset_version: String,
    pub engine: String,
    pub engine_version: String,
    pub config_sha256: String,
    pub code_commit: String,
    pub random_seed: Option<i64>,
    pub timezone: String,
    pub status: String,
    pub summary_json: serde_json::Value,
}

/// The worker's `manifest.json` shape (the Rust contract of the Python
/// `build_manifest` output).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestManifest {
    pub run: RunManifest,
    pub metrics: BTreeMap<String, ReportedStat>,
    pub warnings: Vec<Warning>,
    pub artifacts: Vec<ArtifactManifest>,
}

/// What a [`ManifestWriter::write`] call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReport {
    /// True when rows were inserted; false when the run already existed and
    /// the write was an idempotent no-op.
    pub inserted: bool,
    pub run_id: Uuid,
    pub artifacts: usize,
}

const VALID_RUN_STATUSES: [&str; 5] = ["PENDING", "RUNNING", "SUCCEEDED", "FAILED", "CANCELED"];

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Writes validated backtest manifests into the T3 backtest-result tables.
#[derive(Clone)]
pub struct ManifestWriter {
    pool: PgPool,
}

impl ManifestWriter {
    /// Wraps a pool that must hold the `worker` role grants
    /// (0009_grants: SELECT/INSERT on the backtest tables).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connects a pool from a `DATABASE_URL` (publisher binary entry point).
    pub async fn connect(database_url: &str) -> Result<Self, ManifestError> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await?;
        Ok(Self::new(pool))
    }

    /// Validates a manifest before any database work.
    pub fn validate(manifest: &BacktestManifest) -> Result<(), ManifestError> {
        if !VALID_RUN_STATUSES.contains(&manifest.run.status.as_str()) {
            return Err(ManifestError::Invalid(format!(
                "run.status {:?} is not one of {VALID_RUN_STATUSES:?}",
                manifest.run.status
            )));
        }
        for artifact in &manifest.artifacts {
            if !is_sha256_hex(&artifact.sha256) {
                return Err(ManifestError::Invalid(format!(
                    "artifact {:?} sha256 {:?} is not 64 hex chars",
                    artifact.artifact_type.as_str(),
                    artifact.sha256
                )));
            }
            if artifact.row_count < 0 || artifact.size_bytes < 0 {
                return Err(ManifestError::Invalid(format!(
                    "artifact {:?} has negative row_count/size_bytes",
                    artifact.artifact_type.as_str()
                )));
            }
            if artifact.parquet_path.is_empty() {
                return Err(ManifestError::Invalid(
                    "artifact parquet_path is empty".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Writes the manifest in ONE short transaction. Idempotent: the same run
    /// id re-publishes as a verified no-op.
    pub async fn write(&self, manifest: &BacktestManifest) -> Result<WriteReport, ManifestError> {
        Self::validate(manifest)?;
        let mut tx = self.pool.begin().await?;

        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO backtest_runs (id, owner_user_id, job_id, strategy_id, strategy_version, \
             dataset_version, engine, engine_version, config_sha256, code_commit, random_seed, \
             timezone, status, summary_json) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
             ON CONFLICT (id) DO NOTHING RETURNING id",
        )
        .bind(manifest.run.id)
        .bind(manifest.run.owner_user_id)
        .bind(manifest.run.job_id)
        .bind(&manifest.run.strategy_id)
        .bind(&manifest.run.strategy_version)
        .bind(&manifest.run.dataset_version)
        .bind(&manifest.run.engine)
        .bind(&manifest.run.engine_version)
        .bind(&manifest.run.config_sha256)
        .bind(&manifest.run.code_commit)
        .bind(manifest.run.random_seed)
        .bind(&manifest.run.timezone)
        .bind(&manifest.run.status)
        .bind(&manifest.run.summary_json)
        .fetch_optional(&mut *tx)
        .await?;

        if inserted.is_some() {
            for (key, value) in &manifest.metrics {
                sqlx::query(
                    "INSERT INTO backtest_metrics (backtest_run_id, owner_user_id, metric_key, metric_value) \
                     VALUES ($1, $2, $3, $4::numeric)",
                )
                .bind(manifest.run.id)
                .bind(manifest.run.owner_user_id)
                .bind(key)
                .bind(format!("{value}"))
                .execute(&mut *tx)
                .await?;
            }
            for warning in &manifest.warnings {
                sqlx::query(
                    "INSERT INTO backtest_warnings (backtest_run_id, owner_user_id, warning_code, message) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(manifest.run.id)
                .bind(manifest.run.owner_user_id)
                .bind(&warning.code)
                .bind(&warning.message)
                .execute(&mut *tx)
                .await?;
            }
            for artifact in &manifest.artifacts {
                sqlx::query(
                    "INSERT INTO result_artifacts (backtest_run_id, owner_user_id, artifact_type, \
                     parquet_path, row_count, sha256, size_bytes, summary_json) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                )
                .bind(manifest.run.id)
                .bind(manifest.run.owner_user_id)
                .bind(artifact.artifact_type.as_str())
                .bind(&artifact.parquet_path)
                .bind(artifact.row_count)
                .bind(&artifact.sha256)
                .bind(artifact.size_bytes)
                .bind(&artifact.summary_json)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            return Ok(WriteReport {
                inserted: true,
                run_id: manifest.run.id,
                artifacts: manifest.artifacts.len(),
            });
        }

        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM result_artifacts WHERE backtest_run_id = $1")
                .bind(manifest.run.id)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        if rows as usize != manifest.artifacts.len() {
            return Err(ManifestError::IdempotencyMismatch(format!(
                "run {} already exists with {rows} artifacts, manifest declares {}",
                manifest.run.id,
                manifest.artifacts.len()
            )));
        }
        Ok(WriteReport {
            inserted: false,
            run_id: manifest.run.id,
            artifacts: manifest.artifacts.len(),
        })
    }
}
