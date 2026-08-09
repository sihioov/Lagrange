//! `job-queue` - Lagrange Station PostgreSQL job queue: leased claims,
//! immutable attempts, cooperative cancellation, and orphan recovery.
//!
//! Design §6.8 + requirements NFR-REL-001..003 / FR-BT-008/009, on the frozen
//! T3 schema (`jobs` five-state `jobs_status_check`, `job_attempts` with
//! attempt-level `ORPHANED` and `UNIQUE(job_id, attempt_no)`). See
//! [`queue`] for the lease/cancel/sweep semantics.
//!
//! Package name is `job-queue` (underscore form `job_queue` in `use` paths).

pub mod batch;
pub mod error;
pub mod factor_series;
pub mod paper_execution;
pub mod queue;
pub mod resolver;
pub mod runner;
pub mod types;

pub use batch::{BatchItem, MAX_BATCH_SIZE, cancel_batch, submit_batch};
pub use error::QueueError;
pub use queue::{JobQueue, QueueConfig};
pub use resolver::DbStrategyResolver;
pub use types::{
    AttemptOutcome, AuditActor, CancelResult, ClaimedJob, ErrorClass, HeartbeatStatus, Job,
    JobAttempt, JobStatus, SettleResult, SubmitJob, SweepReport,
};
