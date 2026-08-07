//! Bounded batch submission and cascade cancellation on top of the generic
//! job queue (plan Todo 29 — the job-queue side of robustness suite
//! orchestration).
//!
//! This module knows NOTHING about what a "robustness suite" is: only "N
//! related jobs, one owner, each with a caller-supplied idempotency key."
//! Domain-specific fan-out planning (parameter-grid limits, one-axis
//! children, holdout guards) lives in the crate that owns that domain
//! (`result-model`); `job-queue` stays generic on purpose so it never
//! depends on `result-model`.

use crate::error::QueueError;
use crate::queue::JobQueue;
use crate::types::{AuditActor, CancelResult, Job, SubmitJob};
use sqlx::types::Uuid;

/// Hard cap on jobs submitted by ONE [`submit_batch`] call. Independent of
/// any domain-specific grid limit — no caller may fan out an unbounded
/// number of children through this API.
pub const MAX_BATCH_SIZE: usize = 50;

/// One item of a batch submission. The caller decides `job_type`/`payload`/
/// `idempotency_key` per item; job-queue only bounds and submits them.
#[derive(Debug, Clone)]
pub struct BatchItem {
    pub job_type: String,
    pub payload: serde_json::Value,
    pub idempotency_key: String,
}

/// Submits `items` as independent jobs owned by `owner_user_id`, bounded by
/// [`MAX_BATCH_SIZE`]. An oversized batch is rejected wholesale — before any
/// row lands — never truncated to a partial submission.
///
/// Each item's `idempotency_key` makes re-submission crash-safe: calling
/// this twice with the SAME items resolves to the SAME jobs (per-owner
/// `submit` idempotency, FR-BT-008/AT-03 semantics extended to batches),
/// never duplicates.
pub async fn submit_batch(
    queue: &JobQueue,
    owner_user_id: Uuid,
    items: Vec<BatchItem>,
    priority: i32,
    max_attempts: i32,
) -> Result<Vec<Job>, QueueError> {
    if items.is_empty() {
        return Err(QueueError::InvalidInput(
            "batch must contain at least one item".to_owned(),
        ));
    }
    if items.len() > MAX_BATCH_SIZE {
        return Err(QueueError::InvalidInput(format!(
            "batch of {} items exceeds the maximum of {MAX_BATCH_SIZE}",
            items.len()
        )));
    }
    let mut jobs = Vec::with_capacity(items.len());
    for item in items {
        let job = queue
            .submit(SubmitJob {
                owner_user_id,
                job_type: item.job_type,
                payload: item.payload,
                priority,
                idempotency_key: Some(item.idempotency_key),
                max_attempts,
                available_at: None,
            })
            .await?;
        jobs.push(job);
    }
    Ok(jobs)
}

/// Cascades cancellation to every job in `job_ids`. Best-effort per item:
/// one job's [`QueueError`] does not abort the cascade for its siblings —
/// every id gets an outcome (`Ok` or `Err`) so the caller can decide whether
/// a partial cascade is acceptable. Already-terminal jobs (settled before
/// the cascade fires) come back as [`CancelResult::AlreadyTerminal`] and are
/// left untouched, matching [`JobQueue::request_cancel`]'s own contract.
pub async fn cancel_batch(
    queue: &JobQueue,
    job_ids: &[Uuid],
    actor: &AuditActor,
) -> Vec<(Uuid, Result<CancelResult, QueueError>)> {
    let mut results = Vec::with_capacity(job_ids.len());
    for &job_id in job_ids {
        let outcome = queue.request_cancel(job_id, actor).await;
        results.push((job_id, outcome));
    }
    results
}
