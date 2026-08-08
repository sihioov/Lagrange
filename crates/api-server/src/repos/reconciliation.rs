//! Recording reconciliation runs and deriving account readiness (Todo 40).
//!
//! `kis-client::reconciliation` decides; this records, and answers the one
//! question the Risk Gateway asks: may this account trade?
//!
//! # Green is defined once
//!
//! [`ReconciliationRepo::readiness`] derives its answer from the LATEST run
//! for a connection, and only a run that both completed and recorded zero
//! mismatches counts. It is the same rule `ReconciliationOutcome::is_green`
//! applies in memory; stating it in two places with two meanings would let
//! the reconciler and the gate disagree about whether trading is allowed,
//! which is a disagreement you discover by trading when you should not have.
//!
//! # Nothing is ready by default
//!
//! An account with NO run is not ready. A fresh install, a restored backup,
//! and a process that crashed before its first reconciliation all land there,
//! and FR-LIVE-004 requires every one of them to block.

use crate::actor_tx::begin_actor_tx;
use crate::error::{TenancyError, TenancyResult};
use auth::entitlement::Actor;
use chrono::{DateTime, Utc};
use kis_client::reconciliation::ReconciliationOutcome;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// A recorded run.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ReconciliationRunRow {
    pub id: Uuid,
    pub broker_connection_id: Option<Uuid>,
    pub run_type: String,
    pub status: String,
    pub mismatch_count: i32,
    pub report_path: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

const RUN_COLUMNS: &str = "id, broker_connection_id, run_type, status, mismatch_count, \
     report_path, started_at, finished_at";

/// Whether an account may trade, and why not if it may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// The latest run completed with zero mismatches.
    Ready { run_id: Uuid },
    /// The latest run found differences nobody has resolved.
    Blocked { run_id: Uuid, mismatch_count: i32 },
    /// A run is in progress; its answer is not in yet.
    Running { run_id: Uuid },
    /// No run has ever completed for this connection. Fresh installs,
    /// restored backups and crashed-before-first-run processes all land here,
    /// and all of them must block.
    NeverReconciled,
}

impl Readiness {
    /// The single question the gate asks.
    pub const fn may_trade(&self) -> bool {
        matches!(self, Readiness::Ready { .. })
    }

    pub const fn reason(&self) -> &'static str {
        match self {
            Readiness::Ready { .. } => "READY",
            Readiness::Blocked { .. } => "RECONCILIATION_MISMATCH",
            Readiness::Running { .. } => "RECONCILIATION_IN_PROGRESS",
            Readiness::NeverReconciled => "NEVER_RECONCILED",
        }
    }
}

pub struct ReconciliationRepo {
    pool: PgPool,
    actor: Actor,
    owner_user_id: Uuid,
}

impl ReconciliationRepo {
    pub fn new(pool: PgPool, actor: Actor, owner_user_id: Uuid) -> Self {
        Self {
            pool,
            actor,
            owner_user_id,
        }
    }

    /// Opens a run. Recorded BEFORE the work, so a crash mid-reconciliation
    /// leaves a RUNNING row rather than no trace — and `Running` blocks, so
    /// the crash cannot be mistaken for "never needed one".
    pub async fn start(
        &self,
        connection_id: Option<Uuid>,
        run_type: &str,
    ) -> TenancyResult<ReconciliationRunRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, ReconciliationRunRow>(sqlx::AssertSqlSafe(format!(
            "INSERT INTO reconciliation_runs \
             (owner_user_id, broker_connection_id, run_type, status, started_at) \
             VALUES ($1, $2, $3, 'RUNNING', now()) RETURNING {RUN_COLUMNS}"
        )))
        .bind(self.owner_user_id)
        .bind(connection_id)
        .bind(run_type)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| TenancyError::InvalidState(format!("start run: {e}")))?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;
        Ok(row)
    }

    /// Closes a run with its outcome.
    ///
    /// The mismatch count is the outcome's OWN count, not a re-derivation:
    /// the row must say what the reconciler actually concluded, so that a
    /// later reader is not quietly re-deciding history.
    pub async fn finish(
        &self,
        run_id: Uuid,
        outcome: &ReconciliationOutcome,
        report_path: Option<&str>,
    ) -> TenancyResult<ReconciliationRunRow> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        let row = sqlx::query_as::<_, ReconciliationRunRow>(sqlx::AssertSqlSafe(format!(
            "UPDATE reconciliation_runs \
             SET status = $2, mismatch_count = $3, report_path = $4, finished_at = now() \
             WHERE id = $1 RETURNING {RUN_COLUMNS}"
        )))
        .bind(run_id)
        .bind(if outcome.is_green() {
            "PASSED"
        } else {
            "FAILED"
        })
        .bind(i32::try_from(outcome.mismatches.len()).unwrap_or(i32::MAX))
        .bind(report_path)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| TenancyError::NotFound)?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;
        Ok(row)
    }

    /// The latest run for a connection, whatever its state.
    pub async fn latest(
        &self,
        connection_id: Option<Uuid>,
    ) -> TenancyResult<Option<ReconciliationRunRow>> {
        let mut tx = begin_actor_tx(&self.pool, &self.actor).await?;
        // `IS NOT DISTINCT FROM` so a NULL connection matches a NULL
        // connection; `=` would be NULL and silently match nothing, making
        // every account look never-reconciled.
        let row = sqlx::query_as::<_, ReconciliationRunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM reconciliation_runs \
             WHERE broker_connection_id IS NOT DISTINCT FROM $1 \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        )))
        .bind(connection_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| TenancyError::Forbidden)?;
        tx.commit().await.map_err(|_| TenancyError::Forbidden)?;
        Ok(row)
    }

    /// May this connection trade?
    pub async fn readiness(&self, connection_id: Option<Uuid>) -> TenancyResult<Readiness> {
        Ok(match self.latest(connection_id).await? {
            None => Readiness::NeverReconciled,
            Some(row) => match row.status.as_str() {
                // PASSED alone is not enough: a run that passed while
                // recording mismatches would be a contradiction, and trusting
                // the status over the count would trade through it.
                "PASSED" if row.mismatch_count == 0 => Readiness::Ready { run_id: row.id },
                "PASSED" => Readiness::Blocked {
                    run_id: row.id,
                    mismatch_count: row.mismatch_count,
                },
                "RUNNING" | "PENDING" => Readiness::Running { run_id: row.id },
                _ => Readiness::Blocked {
                    run_id: row.id,
                    mismatch_count: row.mismatch_count,
                },
            },
        })
    }
}

/// Translates readiness into the Risk Gateway's check-5 input.
///
/// The ONLY mapping between the two, so the reconciler and the gate cannot
/// drift apart. Each arm is a deliberate choice about how a denial is graded,
/// because `snapshot::Reconciliation::Unknown` denies as `InputUnavailable`,
/// which §15.3 grades CRITICAL:
///
/// * `Ready`   → `Green`. Trading permitted.
/// * `Blocked` → `NotGreen`. A real, known difference: a policy denial
///   (WARNING), not an incident. Someone must resolve it, but the system is
///   working exactly as designed.
/// * `Running` → `NotGreen` rather than `Unknown`. We DO know the state — a
///   run is in progress — so this is not an absence of information, and
///   paging CRITICAL every time a scheduled reconciliation overlaps an order
///   would be alarm noise that trains people to ignore the grade.
/// * `NeverReconciled` → `Unknown`, and therefore CRITICAL. This one IS an
///   absence of information, and it is the state a fresh install, a restored
///   backup, and a crashed-before-first-run process all land in. Waking
///   someone for it is correct: an account that has never been reconciled has
///   no established relationship to the broker at all, and that is exactly the
///   situation FR-LIVE-004 exists to stop.
pub fn gate_input(readiness: &Readiness) -> kis_client::reconciliation::GateReconciliation {
    use kis_client::reconciliation::GateReconciliation;
    match readiness {
        Readiness::Ready { .. } => GateReconciliation::Green,
        Readiness::Blocked { .. } | Readiness::Running { .. } => GateReconciliation::NotGreen,
        Readiness::NeverReconciled => GateReconciliation::Unknown,
    }
}
